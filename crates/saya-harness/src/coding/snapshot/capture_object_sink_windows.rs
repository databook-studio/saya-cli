//! The Windows write end of one staged object: an exclusive temp inside the
//! staging directory whose `std::io::Write` enforces the stage's aggregate
//! byte bound and keeps a running SHA-256 over every written byte. `finish`
//! flushes the temp, requires the running hash to equal the digest the
//! caller supplies, and only then promotes the temp to
//! `objects/<lowercase-hex-sha256>` with a write-through replace rename.
//! Identical verified content promotes over the same name, so the object
//! store dedups by construction and no caller-supplied path names an object.

use std::{
    fs::{self, File},
    io,
    path::PathBuf,
};

use saya_types::Sha256Digest;
use sha2::{Digest as _, Sha256};

use super::super::{
    SnapshotError, capture_dest::capture_io, capture_publish_windows,
    capture_staging::StagedSnapshot, manifest,
};

/// The write end of one staged object on Windows.
pub struct ObjectSink<'a> {
    owner: &'a mut StagedSnapshot,
    file: Option<File>,
    temp_path: Option<PathBuf>,
    hasher: Sha256,
    written: u64,
    exceeded: bool,
}

impl<'a> ObjectSink<'a> {
    /// Wraps the exclusively-created temp `new_object` opened; visible to
    /// the staging module that constructs sinks.
    pub(in crate::coding::snapshot) fn new(
        owner: &'a mut StagedSnapshot,
        file: File,
        temp_path: PathBuf,
    ) -> Self {
        Self {
            owner,
            file: Some(file),
            temp_path: Some(temp_path),
            hasher: Sha256::new(),
            written: 0,
            exceeded: false,
        }
    }

    /// Flushes the temp, verifies the running hash against the supplied
    /// digest, and promotes the temp under the digest name inside
    /// `objects/`. A sink that exceeded the bound refuses with `TooLarge`; a
    /// digest mismatch refuses with `ObjectDigestMismatch` and removes the
    /// temp, storing nothing under the name; an abandoned sink's temp is
    /// removed by drop.
    pub fn finish(mut self, sha256: &Sha256Digest) -> Result<(), SnapshotError> {
        self.finish_windows(sha256)
    }

    fn finish_windows(&mut self, sha256: &Sha256Digest) -> Result<(), SnapshotError> {
        if self.exceeded {
            return Err(SnapshotError::TooLarge);
        }
        let Some(file) = self.file.take() else {
            return Err(SnapshotError::CaptureIo {
                description: "finish a staged object once".to_string(),
                source: None,
            });
        };
        if let Err(error) = file.sync_all() {
            drop(file);
            self.abandon_temp();
            return Err(capture_io("sync staged object", &error));
        }
        drop(file);
        // The content identity the sink advertises is established here: the
        // hash over every written byte must equal the digest the caller
        // supplied, so a wrong digest can never store bytes under its name.
        let actual = manifest::digest_from_hasher(std::mem::take(&mut self.hasher))?;
        if actual != *sha256 {
            self.abandon_temp();
            return Err(SnapshotError::ObjectDigestMismatch);
        }
        let Some(temp_path) = self.temp_path.take() else {
            return Err(SnapshotError::CaptureIo {
                description: "finish a staged object once".to_string(),
                source: None,
            });
        };
        let target = self.owner.objects_path.join(sha256.as_str());
        // The bytes are verified against the digest that names them, so
        // re-promoting identical content overwrites like with like.
        if let Err(error) = capture_publish_windows::rename_replace(&temp_path, &target) {
            let _ = fs::remove_file(&temp_path);
            return Err(capture_io("promote staged object", &error));
        }
        self.owner.remaining = self.owner.remaining.saturating_sub(self.written);
        self.owner.next_object += 1;
        Ok(())
    }

    /// Removes the temp on a failed finish so a retry does not accumulate
    /// garbage inside its own staging directory.
    fn abandon_temp(&mut self) {
        if let Some(path) = self.temp_path.take() {
            let _ = fs::remove_file(path);
        }
    }
}

impl io::Write for ObjectSink<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // All-or-nothing per call, mirroring the bounded manifest writer: a
        // write that would cross the aggregate bound is refused whole.
        if self.exceeded || self.written.saturating_add(buf.len() as u64) > self.owner.remaining {
            self.exceeded = true;
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "staged snapshot byte bound exceeded",
            ));
        }
        let Some(file) = self.file.as_mut() else {
            return Err(io::Error::other("staged object sink already finished"));
        };
        let written = file.write(buf)?;
        self.hasher.update(&buf[..written]);
        self.written += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.file.as_mut() {
            Some(file) => file.flush(),
            None => Err(io::Error::other("staged object sink already finished")),
        }
    }
}

impl Drop for ObjectSink<'_> {
    fn drop(&mut self) {
        // After a successful promote the temp path is gone, so this only
        // ever removes an abandoned temp; failures leave sweepable garbage.
        if self.file.is_some() {
            self.abandon_temp();
        }
    }
}
