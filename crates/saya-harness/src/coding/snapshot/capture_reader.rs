//! The same-open working-tree capture reader: one planned entry, one
//! contained open, one streaming digest. The reader revalidates the plan's
//! source identity against a fresh no-follow scan (device, inode, size, file
//! type, and the `mode & 0o111` bits), opens exactly once through the
//! anchored no-follow seam against the plan's (dev, inode), streams 64 KiB
//! chunks into the caller's sink while hashing, and re-verifies the identity
//! after the last byte. Never `fs::read`, canonicalize, or a second open —
//! the building block the capture orchestrator composes per entry; it owns
//! no destination, no staging, and no Git blob path.
//!
//! On ANY error the sink may already hold partial bytes: the caller must
//! discard the sink. The staging-directory contract owned elsewhere in
//! capture guarantees a refused capture never presents as complete.

use std::fmt;

use saya_types::Sha256Digest;

use super::{
    manifest::{RedactedIoError, SnapshotError},
    types::SnapshotFileEntry,
};

#[cfg(unix)]
#[path = "capture_reader_error.rs"]
mod reader_error;

/// The streaming chunk size for read-and-hash.
#[cfg(unix)]
const CHUNK_BYTES: usize = 64 * 1024;

/// One captured working-tree file: the streamed SHA-256 digest and byte
/// count. Debug prints only the count and the digest — never a path.
pub struct CapturedFile {
    sha256: Sha256Digest,
    bytes: u64,
}

impl CapturedFile {
    /// The lowercase SHA-256 of the captured bytes.
    pub fn sha256(&self) -> &Sha256Digest {
        &self.sha256
    }

    /// The captured byte count, equal to the plan's declared size on success.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

impl fmt::Debug for CapturedFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CapturedFile")
            .field("bytes", &self.bytes)
            .field("sha256", &self.sha256)
            .finish()
    }
}

/// Fails closed on hosts without the anchored walk instead of compiling the
/// reader away: capture is unix-only by the same gate `GitService::new` uses.
#[cfg(not(unix))]
mod non_unix_reader {
    use std::io::Write;

    use saya_agent::CancellationToken;

    use crate::workspace::Workspace;

    use super::{CapturedFile, RedactedIoError, SnapshotError, SnapshotFileEntry};

    pub fn read_working_file(
        workspace: &Workspace,
        entry: &SnapshotFileEntry,
        remaining_aggregate_bytes: u64,
        sink: &mut dyn Write,
        cancellation: &CancellationToken,
    ) -> Result<CapturedFile, SnapshotError> {
        let _ = (workspace, entry);
        let _ = (remaining_aggregate_bytes, sink, cancellation);
        Err(SnapshotError::CaptureIo {
            description: "snapshot capture requires a unix host".to_owned(),
            source: Some(RedactedIoError::new(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "non-unix host",
            ))),
        })
    }
}

#[cfg(not(unix))]
pub use non_unix_reader::read_working_file;

#[cfg(unix)]
mod unix_reader {
    use std::{fs::File, io::Read, io::Write};

    use saya_agent::CancellationToken;
    use sha2::{Digest, Sha256};

    use crate::workspace::{Workspace, anchor::Anchor, fd::FinalStat};

    use super::super::types::{
        MAX_SNAPSHOT_FILE_BYTES, SnapshotContentSource, SnapshotFileMode, SnapshotSourceIdentity,
    };
    use super::{
        CHUNK_BYTES, CapturedFile, Sha256Digest, SnapshotError, SnapshotFileEntry,
        reader_error::{capture_io, refuse, source_io},
    };

    /// Reads one planned working-tree file in a single contained open,
    /// streaming it into `sink` while hashing. The per-file 64 MiB ceiling
    /// and `remaining_aggregate_bytes` are enforced on the cumulative bytes
    /// during the stream; a budget below the planned size refuses before the
    /// open. See the module docs for the identity and partial-sink rules.
    pub fn read_working_file(
        workspace: &Workspace,
        entry: &SnapshotFileEntry,
        remaining_aggregate_bytes: u64,
        sink: &mut dyn Write,
        cancellation: &CancellationToken,
    ) -> Result<CapturedFile, SnapshotError> {
        if !matches!(entry.source(), SnapshotContentSource::WorkingTree) {
            return Err(SnapshotError::SourceNotWorkingTree);
        }
        let Some(identity) = entry.source_identity() else {
            // Without the plan's pinned identity the required match cannot
            // hold; refuse rather than read unpinned content.
            return Err(SnapshotError::SourceIdentityChanged);
        };
        if cancellation.is_cancelled() {
            return Err(SnapshotError::Cancelled);
        }
        let declared = identity.size();
        if declared > MAX_SNAPSHOT_FILE_BYTES || declared > remaining_aggregate_bytes {
            return Err(SnapshotError::TooLarge);
        }
        let path = entry.path().as_str();
        let anchor = match workspace.anchor(path, false) {
            Ok(anchor) => anchor,
            Err(error) => return Err(refuse("scan the snapshot source", error)),
        };
        let Some(stat) = anchor.stat() else {
            return Err(SnapshotError::SourceDisappeared);
        };
        verify_scan(stat, entry, identity)?;
        let mut file = match anchor.open_verified((identity.device, identity.inode), path) {
            Ok(file) => file,
            Err(error) => return Err(refuse("open the snapshot source", error)),
        };
        let (hasher, scanned) = stream_captured(
            &mut file,
            identity,
            remaining_aggregate_bytes,
            sink,
            cancellation,
        )?;
        verify_final(&anchor, entry, identity, scanned)?;
        Ok(CapturedFile {
            sha256: digest_of(hasher)?,
            bytes: scanned,
        })
    }

    /// The pre-open scan must still be the planned file: same type, same
    /// (dev, ino), size, and `mode & 0o111` shape — never the full `st_mode`
    /// (umask noise).
    fn verify_scan(
        stat: &FinalStat,
        entry: &SnapshotFileEntry,
        identity: &SnapshotSourceIdentity,
    ) -> Result<(), SnapshotError> {
        if !stat.is_file() {
            return Err(if stat.is_symlink() {
                SnapshotError::SourceSymlink
            } else {
                SnapshotError::SourceNotRegularFile
            });
        }
        if stat.identity() != (identity.device, identity.inode) || stat.len() != identity.size() {
            return Err(SnapshotError::SourceIdentityChanged);
        }
        if stat.has_exec_bits() != (entry.mode() == SnapshotFileMode::Executable) {
            return Err(SnapshotError::SourceIdentityChanged);
        }
        Ok(())
    }

    /// Streams in 64 KiB chunks, hashing and writing each one. Growth past
    /// the declared size, an early EOF, either cap, or cancellation between
    /// chunks all stop the stream; the sink may hold a prefix on any return.
    fn stream_captured(
        file: &mut File,
        identity: &SnapshotSourceIdentity,
        remaining_aggregate_bytes: u64,
        sink: &mut dyn Write,
        cancellation: &CancellationToken,
    ) -> Result<(Sha256, u64), SnapshotError> {
        let declared = identity.size();
        let mut hasher = Sha256::new();
        let mut chunk = [0u8; CHUNK_BYTES];
        let mut scanned = 0u64;
        loop {
            if cancellation.is_cancelled() {
                return Err(SnapshotError::Cancelled);
            }
            let read = file
                .read(&mut chunk)
                .map_err(|error| capture_io("read the snapshot source", error))?;
            if read == 0 {
                break;
            }
            scanned = scanned
                .checked_add(read as u64)
                .ok_or(SnapshotError::TooLarge)?;
            if scanned > declared {
                return Err(SnapshotError::SourceIdentityChanged);
            }
            if scanned > MAX_SNAPSHOT_FILE_BYTES || scanned > remaining_aggregate_bytes {
                return Err(SnapshotError::TooLarge);
            }
            hasher.update(&chunk[..read]);
            sink.write_all(&chunk[..read])
                .map_err(|error| capture_io("write the captured chunk", error))?;
        }
        if scanned != declared {
            return Err(SnapshotError::SourceIdentityChanged);
        }
        Ok((hasher, scanned))
    }

    /// The post-read stat must still be the planned file and the bytes
    /// read: no growth, shrink, inode swap, or mode flip after the stream.
    fn verify_final(
        anchor: &Anchor,
        entry: &SnapshotFileEntry,
        identity: &SnapshotSourceIdentity,
        scanned: u64,
    ) -> Result<(), SnapshotError> {
        let stat = anchor
            .stat_final()
            .map_err(|error| source_io("verify the captured source", error))?;
        if stat.identity() != (identity.device, identity.inode)
            || !stat.is_file()
            || stat.len() != scanned
            || stat.has_exec_bits() != (entry.mode() == SnapshotFileMode::Executable)
        {
            return Err(SnapshotError::SourceIdentityChanged);
        }
        Ok(())
    }

    fn digest_of(hasher: Sha256) -> Result<Sha256Digest, SnapshotError> {
        let hex: String = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Sha256Digest::parse(&hex).map_err(|_| SnapshotError::Serialization)
    }
}

#[cfg(unix)]
pub use unix_reader::read_working_file;
