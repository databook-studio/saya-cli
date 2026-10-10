//! The write end of one staged object: an exclusive 0600 temp inside the
//! staging directory whose `std::io::Write` enforces the stage's aggregate
//! byte bound and keeps a running SHA-256 over every written byte. `finish`
//! fsyncs the temp, requires the running hash to equal the digest the
//! caller supplies, and only then promotes the temp to its content-addressed
//! name `objects/<lowercase-hex-sha256>`. Identical verified content
//! promotes over the same name, so the object store dedups by construction
//! and no caller-supplied path can ever name an object.

use saya_types::Sha256Digest;

use super::{SnapshotError, capture_staging::StagedSnapshot};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::{
    ffi::{OsStr, OsString},
    fs, io,
    io::Write as _,
    os::fd::AsRawFd,
};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use sha2::{Digest as _, Sha256};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use super::{capture_dest::capture_io, manifest};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use crate::workspace::fd::{rename_at, unlink_at};

/// The write end of one staged object, on the targets where staging
/// exists.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub struct ObjectSink<'a> {
    owner: &'a mut StagedSnapshot,
    file: Option<fs::File>,
    temp_name: OsString,
    hasher: Sha256,
    written: u64,
    exceeded: bool,
}

/// Elsewhere the sink is only ever a refusal handle: `begin` and
/// `new_object` already refuse, so no sink is ever constructed and the
/// carried lifetime is all that remains.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub struct ObjectSink<'a> {
    _owner: std::marker::PhantomData<&'a mut StagedSnapshot>,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl<'a> ObjectSink<'a> {
    /// Wraps the exclusively-created temp `new_object` opened; `pub(super)`
    /// because only the staged capture constructs sinks.
    pub(super) fn new(owner: &'a mut StagedSnapshot, file: fs::File, temp_name: OsString) -> Self {
        Self {
            owner,
            file: Some(file),
            temp_name,
            hasher: Sha256::new(),
            written: 0,
            exceeded: false,
        }
    }

    /// Fsyncs the temp, verifies the running hash against the supplied
    /// digest, and renames the temp from its staging directory under the
    /// digest name inside `objects/`. A sink that exceeded the bound refuses
    /// with `TooLarge`; a digest mismatch refuses with `ObjectDigestMismatch`
    /// and unlinks the temp, storing nothing under the name; an abandoned
    /// sink's temp is unlinked by drop.
    pub fn finish(mut self, sha256: &Sha256Digest) -> Result<(), SnapshotError> {
        self.finish_unix(sha256)
    }

    fn finish_unix(&mut self, sha256: &Sha256Digest) -> Result<(), SnapshotError> {
        if self.exceeded {
            return Err(SnapshotError::TooLarge);
        }
        let file = self.file.take().ok_or_else(|| SnapshotError::CaptureIo {
            description: "finish a staged object once".to_string(),
            source: None,
        })?;
        if let Err(error) = file.sync_all() {
            drop(file);
            self.abandon_temp();
            return Err(capture_io("sync staged object", &error));
        }
        // The content identity the sink advertises is established here: the
        // hash over every written byte must equal the digest the caller
        // supplied, so a wrong digest can never store bytes under its name.
        let actual = manifest::digest_from_hasher(std::mem::take(&mut self.hasher))?;
        if actual != *sha256 {
            self.abandon_temp();
            return Err(SnapshotError::ObjectDigestMismatch);
        }
        // Anchored single-component rename from the staging directory into
        // `objects/`; the digest is the name and the bytes are verified, so
        // re-promoting identical content replaces like with like and
        // nothing user-supplied can name an object.
        let promoted = rename_at(
            self.owner.staging_dir.as_raw_fd(),
            &self.temp_name,
            self.owner.objects_dir.as_raw_fd(),
            OsStr::new(sha256.as_str()),
        );
        if let Err(error) = promoted {
            self.abandon_temp();
            return Err(capture_io("promote staged object", &error));
        }
        self.owner.remaining = self.owner.remaining.saturating_sub(self.written);
        self.owner.next_object += 1;
        Ok(())
    }

    /// Removes the temp on a failed finish so a retry does not accumulate
    /// garbage inside its own staging directory.
    fn abandon_temp(&mut self) {
        let _ = unlink_at(self.owner.staging_dir.as_raw_fd(), &self.temp_name);
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
impl<'a> ObjectSink<'a> {
    /// Refuses: there is no staged capture to finish on this target.
    pub fn finish(self, _sha256: &Sha256Digest) -> Result<(), SnapshotError> {
        Err(SnapshotError::ArtifactRootRefused)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
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

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl Drop for ObjectSink<'_> {
    fn drop(&mut self) {
        // After a successful promote the temp name is gone, so this only
        // ever removes an abandoned temp; failures leave sweepable garbage.
        if self.file.is_some() {
            let _ = unlink_at(self.owner.staging_dir.as_raw_fd(), &self.temp_name);
        }
    }
}
