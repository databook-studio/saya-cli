//! The walk's hashing and bound accounting: one file or symlink at a time,
//! with every read re-checking the gate. Kept apart from the directory walk so
//! the traversal and the byte-level rules stay separate concerns.

use std::{
    ffi::OsStr,
    io::{ErrorKind, Read},
    os::fd::RawFd,
    os::unix::fs::PermissionsExt,
    time::Instant,
};

use saya_agent::CancellationToken;
use saya_types::Sha256Digest;
use sha2::{Digest, Sha256};

use super::super::delta_types::{
    DeltaError, MAX_DELTA_NOT_INCLUDED, NotIncludedPath, NotIncludedReason,
};
use crate::coding::snapshot::{
    MAX_SNAPSHOT_FILE_BYTES, MAX_SNAPSHOT_FILES, MAX_SNAPSHOT_TOTAL_BYTES,
    MAX_SYMLINK_TARGET_BYTES, SnapshotFileMode, SymlinkSet, validate_symlink_target,
};
use crate::workspace::fd::{open_file_at, read_link_at};

/// Records one path the walk could not include. A representable refusal is
/// absorbed; anything else is returned so the caller can stop or refuse.
pub(super) fn record(
    state: &mut WalkState,
    path: &str,
    error: DeltaError,
) -> Result<(), DeltaError> {
    match error {
        DeltaError::FileTooLarge => state.not_included(path, NotIncludedReason::Oversize),
        DeltaError::Symlink => state.not_included(path, NotIncludedReason::SpecialFile),
        DeltaError::UnsafePath => state.not_included(path, NotIncludedReason::UnsafePath),
        other => return Err(other),
    }
    Ok(())
}

/// The running bounds the walk enforces: files counted and bytes actually
/// read (never a stale lstat length), plus the bounded not-included report.
#[derive(Default)]
pub(super) struct WalkState {
    pub(super) total_bytes: u64,
    pub(super) file_count: usize,
    pub(super) links: SymlinkSet,
    pub(super) not_included: Vec<NotIncludedPath>,
    pub(super) not_included_overflow: usize,
}

impl WalkState {
    pub(super) fn not_included(&mut self, path: &str, reason: NotIncludedReason) {
        if self.not_included.len() < MAX_DELTA_NOT_INCLUDED {
            self.not_included
                .push(NotIncludedPath::new(path.to_owned(), reason));
        } else {
            self.not_included_overflow = self.not_included_overflow.saturating_add(1);
        }
    }

    pub(super) fn commit_file(&mut self, bytes: u64) {
        self.file_count = self.file_count.saturating_add(1);
        self.total_bytes = self.total_bytes.saturating_add(bytes);
    }
}

/// Opens and hashes one file relative to its parent descriptor. The fstat
/// comes from the opened file, so a swap after the entry stat cannot change
/// the type, the size, or the executable bit; every read re-checks the gate,
/// and the aggregate is committed only after the file succeeds.
pub(super) fn hash_file(
    dir_fd: RawFd,
    name: &OsStr,
    cancellation: &CancellationToken,
    deadline: Instant,
    state: &WalkState,
) -> Result<(u64, Sha256Digest, SnapshotFileMode), DeltaError> {
    if state.file_count >= MAX_SNAPSHOT_FILES {
        return Err(DeltaError::TooManyFiles);
    }
    let mut file =
        open_file_at(dir_fd, name, libc::O_RDONLY | libc::O_NONBLOCK, 0).map_err(|error| {
            match error.raw_os_error() {
                Some(libc::ELOOP) => DeltaError::Symlink,
                _ => DeltaError::Io,
            }
        })?;
    let meta = file.metadata().map_err(|_| DeltaError::Io)?;
    if !meta.is_file() {
        return Err(DeltaError::Symlink);
    }
    if meta.len() > MAX_SNAPSHOT_FILE_BYTES {
        return Err(DeltaError::FileTooLarge);
    }
    let mode = if meta.permissions().mode() & 0o111 != 0 {
        SnapshotFileMode::Executable
    } else {
        SnapshotFileMode::Regular
    };
    let limit = MAX_SNAPSHOT_FILE_BYTES.saturating_add(1);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; READ_CHUNK_BYTES];
    let mut read_total: u64 = 0;
    loop {
        check_gate(cancellation, deadline)?;
        let remaining = limit - read_total;
        if remaining == 0 {
            return Err(DeltaError::FileTooLarge);
        }
        let want = remaining.min(READ_CHUNK_BYTES as u64) as usize;
        match file.read(&mut buffer[..want]) {
            Ok(0) => break,
            Ok(read) => {
                hasher.update(&buffer[..read]);
                read_total += read as u64;
                let total = state
                    .total_bytes
                    .checked_add(read_total)
                    .ok_or(DeltaError::TooLarge)?;
                if total > MAX_SNAPSHOT_TOTAL_BYTES {
                    return Err(DeltaError::TooLarge);
                }
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return Err(DeltaError::Io),
        }
    }
    check_gate(cancellation, deadline)?;
    let hex = hex_lower(&hasher.finalize());
    let sha256 = Sha256Digest::parse(&hex).map_err(|_| DeltaError::Io)?;
    Ok((read_total, sha256, mode))
}

/// Hashes one symlink's target bytes without following the link. The target
/// is bounded like a captured one, must resolve lexically inside the root,
/// counts toward the file and byte bounds, and never escapes the walk.
pub(super) fn hash_symlink(
    dir_fd: RawFd,
    name: &OsStr,
    path: &str,
    cancellation: &CancellationToken,
    deadline: Instant,
    state: &mut WalkState,
) -> Result<(u64, Sha256Digest), DeltaError> {
    if state.file_count >= MAX_SNAPSHOT_FILES {
        return Err(DeltaError::TooManyFiles);
    }
    check_gate(cancellation, deadline)?;
    let target =
        read_link_at(dir_fd, name, MAX_SYMLINK_TARGET_BYTES).map_err(|error| {
            match error.kind() {
                ErrorKind::InvalidData => DeltaError::FileTooLarge,
                _ => DeltaError::Io,
            }
        })?;
    validate_symlink_target(path, &target).map_err(|_| DeltaError::UnsafePath)?;
    let bytes = target.len() as u64;
    let total = state
        .total_bytes
        .checked_add(bytes)
        .ok_or(DeltaError::TooLarge)?;
    if total > MAX_SNAPSHOT_TOTAL_BYTES {
        return Err(DeltaError::TooLarge);
    }
    check_gate(cancellation, deadline)?;
    state.links.insert(path, target.clone());
    state.commit_file(bytes);
    let hex = hex_lower(&Sha256::digest(&target));
    let sha256 = Sha256Digest::parse(&hex).map_err(|_| DeltaError::Io)?;
    Ok((bytes, sha256))
}

/// The walk's one gate: cancelled beats expired, and both beat success.
pub(super) fn check_gate(
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<(), DeltaError> {
    if cancellation.is_cancelled() {
        return Err(DeltaError::Cancelled);
    }
    if Instant::now() >= deadline {
        return Err(DeltaError::DeadlineExceeded);
    }
    Ok(())
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        hex.push(HEX_DIGITS[(byte >> 4) as usize] as char);
        hex.push(HEX_DIGITS[(byte & 0x0f) as usize] as char);
    }
    hex
}

const READ_CHUNK_BYTES: usize = 64 * 1024;
