//! The live-checkout walk behind every delta: it fingerprints the files the
//! workspace currently holds. The Unix implementation is anchored — every
//! open is relative to a descriptor for the entry's parent with `O_NOFOLLOW`,
//! so a swapped symlink is refused instead of followed.

use std::path::Path;
use std::time::Instant;

use saya_agent::CancellationToken;

use super::delta_types::{DeltaError, LiveFile};

#[cfg(unix)]
use crate::coding::snapshot::{
    MAX_SNAPSHOT_FILE_BYTES, MAX_SNAPSHOT_FILES, MAX_SNAPSHOT_TOTAL_BYTES, SnapshotPath,
};
#[cfg(unix)]
use crate::workspace::fd::{DirStream, open_file_at, open_root_fd, stat_at};
#[cfg(unix)]
use saya_types::Sha256Digest;
#[cfg(unix)]
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::{
    ffi::OsStr,
    io::{ErrorKind, Read},
    os::fd::{AsRawFd, RawFd},
    os::unix::fs::PermissionsExt,
};

/// Walks the checkout rooted at `root` and returns every live file, sorted by
/// path. Every access is relative to a descriptor for its parent and
/// no-follow, so a swapped symlink is refused instead of followed; sizes and
/// the executable bit come from the opened file; bounds count bytes actually
/// read. Descent is depth-first, so only the ancestor directory streams are
/// open.
#[cfg(unix)]
pub(super) fn walk_checkout(
    root: &Path,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<Vec<LiveFile>, DeltaError> {
    let root_fd = open_root_fd(root.as_os_str()).map_err(|_| DeltaError::Io)?;
    let root_stream =
        DirStream::open_at(root_fd.as_raw_fd(), OsStr::new(".")).map_err(|_| DeltaError::Io)?;
    let mut stack: Vec<(DirStream, String)> = vec![(root_stream, String::new())];
    let mut files: Vec<LiveFile> = Vec::new();
    let mut state = WalkState::default();

    while let Some((stream, prefix)) = stack.last_mut() {
        check_gate(cancellation, deadline)?;
        let Some((name, _d_type)) = stream.next().map_err(|_| DeltaError::Io)? else {
            stack.pop();
            continue;
        };
        let name = match name.to_str() {
            Some(name) => name.to_owned(),
            None => return Err(DeltaError::UnsafePath),
        };
        let is_root = prefix.is_empty();
        let dir_fd = stream.dir_fd();
        if name.eq_ignore_ascii_case(".git") {
            if is_root
                && name == ".git"
                && stat_at(dir_fd, OsStr::new(&name))
                    .map_err(|_| DeltaError::Io)?
                    .is_file()
            {
                continue;
            }
            return Err(DeltaError::NestedRepository);
        }
        let relative = if is_root {
            name.clone()
        } else {
            format!("{}/{}", prefix.as_str(), name)
        };
        SnapshotPath::parse(relative.clone()).map_err(|_| DeltaError::UnsafePath)?;
        let st = stat_at(dir_fd, OsStr::new(&name)).map_err(|_| DeltaError::Io)?;
        if st.is_dir() {
            if stack.len() >= MAX_WALK_DEPTH {
                return Err(DeltaError::UnsafePath);
            }
            let child =
                DirStream::open_at(dir_fd, OsStr::new(&name)).map_err(|error| {
                    match error.raw_os_error() {
                        Some(libc::ELOOP) | Some(libc::ENOTDIR) => DeltaError::Symlink,
                        _ => DeltaError::Io,
                    }
                })?;
            stack.push((child, relative));
        } else if st.is_file() {
            let (bytes, sha256, executable) = hash_file(
                dir_fd,
                OsStr::new(&name),
                cancellation,
                deadline,
                &mut state,
            )?;
            files.push(LiveFile {
                path: relative,
                executable,
                bytes,
                sha256,
            });
        } else {
            return Err(DeltaError::Symlink);
        }
    }

    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

/// The running bounds the walk enforces: files counted and bytes actually
/// read (never a stale lstat length).
#[cfg(unix)]
#[derive(Default)]
struct WalkState {
    total_bytes: u64,
    file_count: usize,
}

/// Opens and hashes one file relative to its parent descriptor. The fstat
/// comes from the opened file, so a swap after the entry stat cannot change
/// the type, the size, or the executable bit; every read re-checks the gate,
/// and every chunk's bytes join the aggregate before the file can succeed.
#[cfg(unix)]
fn hash_file(
    dir_fd: RawFd,
    name: &OsStr,
    cancellation: &CancellationToken,
    deadline: Instant,
    state: &mut WalkState,
) -> Result<(u64, Sha256Digest, bool), DeltaError> {
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
    let executable = meta.permissions().mode() & 0o111 != 0;
    state.file_count += 1;
    if state.file_count > MAX_SNAPSHOT_FILES {
        return Err(DeltaError::TooManyFiles);
    }
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
                state.total_bytes = state
                    .total_bytes
                    .checked_add(read as u64)
                    .ok_or(DeltaError::TooLarge)?;
                if state.total_bytes > MAX_SNAPSHOT_TOTAL_BYTES {
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
    Ok((read_total, sha256, executable))
}

/// The walk's one gate: cancelled beats expired, and both beat success.
#[cfg(unix)]
fn check_gate(cancellation: &CancellationToken, deadline: Instant) -> Result<(), DeltaError> {
    if cancellation.is_cancelled() {
        return Err(DeltaError::Cancelled);
    }
    if Instant::now() >= deadline {
        return Err(DeltaError::DeadlineExceeded);
    }
    Ok(())
}

#[cfg(unix)]
fn hex_lower(bytes: &[u8]) -> String {
    const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        hex.push(HEX_DIGITS[(byte >> 4) as usize] as char);
        hex.push(HEX_DIGITS[(byte & 0x0f) as usize] as char);
    }
    hex
}

#[cfg(unix)]
const READ_CHUNK_BYTES: usize = 64 * 1024;

/// A directory nesting deeper than this bound is refused as an unsafe path,
/// and it keeps the ancestor directory streams well under the process
/// descriptor limit.
#[cfg(unix)]
const MAX_WALK_DEPTH: usize = 64;

/// Non-Unix hosts have no anchored walk — no descriptor-relative, no-follow
/// opens to lean on — and the coding feature is Unix-only today, so the walk
/// refuses.
#[cfg(not(unix))]
pub(super) fn walk_checkout(
    root: &Path,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<Vec<LiveFile>, DeltaError> {
    let _ = (root, cancellation, deadline);
    Err(DeltaError::Io)
}
