//! The read-only, anchored probe behind the apply-back preflight: for each
//! accepted delta path it decides whether the user's original checkout has
//! drifted from the capture-time baseline. Every open is relative to the
//! borrowed root descriptor with `O_NOFOLLOW`, so a swapped symlink is a
//! typed collision instead of a followed path, and nothing here ever writes.

use std::time::Instant;

use saya_agent::CancellationToken;
use saya_types::Sha256Digest;

use super::preflight::{CollisionKind, PreflightError};

#[cfg(not(unix))]
use crate::coding::snapshot::SnapshotFileMode;
#[cfg(unix)]
use crate::coding::snapshot::{
    MAX_SNAPSHOT_FILE_BYTES, MAX_SNAPSHOT_FILES, MAX_SNAPSHOT_TOTAL_BYTES,
    MAX_SYMLINK_TARGET_BYTES, SnapshotFileMode, SymlinkSet,
};
#[cfg(unix)]
use crate::workspace::fd::{
    DirStream, open_dir_at, open_file_at, open_root_fd, read_link_at, stat_at,
};
#[cfg(unix)]
use sha2::{Digest, Sha256};
#[cfg(not(unix))]
use std::os::raw::c_int as RawFd;
#[cfg(unix)]
use std::{
    ffi::OsStr,
    io::{ErrorKind, Read},
    os::fd::{AsRawFd, OwnedFd, RawFd},
    os::unix::fs::PermissionsExt,
    path::Path,
};

/// The preflight's running bounds state: bytes actually read, joined across
/// every classified path and never a stale length.
#[derive(Default)]
pub(super) struct ReadState {
    pub(super) total_bytes: u64,
}

/// Classifies one accepted delta path against the baseline entry it had at
/// capture — `(content_sha256, mode)` when the path existed, `None` when it
/// did not. Missing where the baseline has an entry is `UserDeleted`, present
/// where it has none is `UserAddedUntracked`, a non-regular file or a swapped
/// symlink is `TypeChanged`, and content, mode, or symlink-target drift is
/// `UserEdited`. Bytes join `state.total_bytes` under the size bounds, the
/// gate is re-checked before every read, and the tree is never written.
#[cfg(unix)]
pub(super) fn classify_worktree(
    root_fd: RawFd,
    rel: &str,
    baseline: Option<(&Sha256Digest, SnapshotFileMode)>,
    state: &mut ReadState,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<Option<CollisionKind>, PreflightError> {
    check_gate(cancellation, deadline)?;
    let mut components = rel.split('/');
    let last = components.next_back().unwrap_or_default();
    let mut parent = root_fd;
    let mut held: Option<OwnedFd>;
    for component in components {
        held = Some(match open_dir_at(parent, OsStr::new(component)) {
            Ok(next) => next,
            Err(error) => match error.raw_os_error() {
                Some(libc::ENOENT) => {
                    return Ok(if baseline.is_some() {
                        Some(CollisionKind::UserDeleted)
                    } else {
                        None
                    });
                }
                Some(libc::ELOOP) | Some(libc::ENOTDIR) => {
                    return Ok(Some(CollisionKind::TypeChanged));
                }
                _ => return Err(PreflightError::Io),
            },
        });
        if let Some(current) = held.as_ref() {
            parent = current.as_raw_fd();
        }
    }
    let st = match stat_at(parent, OsStr::new(last)) {
        Ok(st) => st,
        Err(error) => match error.raw_os_error() {
            Some(libc::ENOENT) => {
                return Ok(if baseline.is_some() {
                    Some(CollisionKind::UserDeleted)
                } else {
                    None
                });
            }
            _ => return Err(PreflightError::Io),
        },
    };
    let Some((baseline_digest, baseline_mode)) = baseline else {
        return Ok(Some(if st.is_file() || st.is_symlink() {
            CollisionKind::UserAddedUntracked
        } else {
            CollisionKind::TypeChanged
        }));
    };
    if st.is_symlink() {
        if baseline_mode != SnapshotFileMode::Symlink {
            return Ok(Some(CollisionKind::TypeChanged));
        }
        let target =
            read_link_at(parent, OsStr::new(last), MAX_SYMLINK_TARGET_BYTES).map_err(|error| {
                match error.kind() {
                    ErrorKind::InvalidData => PreflightError::Oversize,
                    _ => PreflightError::Io,
                }
            })?;
        state.total_bytes = state
            .total_bytes
            .checked_add(target.len() as u64)
            .ok_or(PreflightError::Oversize)?;
        if state.total_bytes > MAX_SNAPSHOT_TOTAL_BYTES {
            return Err(PreflightError::Oversize);
        }
        let digest = Sha256Digest::parse(&hex_lower(&Sha256::digest(&target)))
            .map_err(|_| PreflightError::Io)?;
        return Ok((digest != *baseline_digest).then_some(CollisionKind::UserEdited));
    }
    if !st.is_file() || baseline_mode == SnapshotFileMode::Symlink {
        return Ok(Some(CollisionKind::TypeChanged));
    }
    let mut file = match open_file_at(
        parent,
        OsStr::new(last),
        libc::O_RDONLY | libc::O_NONBLOCK,
        0,
    ) {
        Ok(file) => file,
        Err(error) => match error.raw_os_error() {
            Some(libc::ELOOP) => return Ok(Some(CollisionKind::TypeChanged)),
            _ => return Err(PreflightError::Io),
        },
    };
    let meta = file.metadata().map_err(|_| PreflightError::Io)?;
    if !meta.is_file() {
        return Ok(Some(CollisionKind::TypeChanged));
    }
    if meta.len() > MAX_SNAPSHOT_FILE_BYTES {
        return Err(PreflightError::Oversize);
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
            return Err(PreflightError::Oversize);
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
                    .ok_or(PreflightError::Oversize)?;
                if state.total_bytes > MAX_SNAPSHOT_TOTAL_BYTES {
                    return Err(PreflightError::Oversize);
                }
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return Err(PreflightError::Io),
        }
    }
    check_gate(cancellation, deadline)?;
    let hex = hex_lower(&hasher.finalize());
    let digest = Sha256Digest::parse(&hex).map_err(|_| PreflightError::Io)?;
    if digest != *baseline_digest || mode != baseline_mode {
        return Ok(Some(CollisionKind::UserEdited));
    }
    Ok(None)
}

/// Collects every symlink in the job's candidate checkout and validates the
/// whole set chain-aware. The walk is anchored and no-follow; regular files
/// are skipped (the delta walk hashes them), `.git` and the `.saya` output
/// root are skipped, and the same depth, count, and byte bounds apply.
#[cfg(unix)]
pub(super) fn validate_candidate_symlinks(
    root: &Path,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<(), PreflightError> {
    let root_fd = open_root_fd(root.as_os_str()).map_err(|_| PreflightError::Io)?;
    let root_stream =
        DirStream::open_at(root_fd.as_raw_fd(), OsStr::new(".")).map_err(|_| PreflightError::Io)?;
    let mut stack: Vec<(DirStream, String)> = vec![(root_stream, String::new())];
    let mut links = SymlinkSet::default();
    let mut count = 0usize;
    let mut total = 0u64;
    while let Some((stream, prefix)) = stack.last_mut() {
        check_gate(cancellation, deadline)?;
        let Some((name, _)) = stream.next().map_err(|_| PreflightError::Io)? else {
            stack.pop();
            continue;
        };
        let name = name.to_str().ok_or(PreflightError::Io)?;
        let is_root = prefix.is_empty();
        if name.eq_ignore_ascii_case(".git")
            || (is_root && name == super::delta_output::OUTPUT_ROOT_NAME)
        {
            continue;
        }
        let relative = if is_root {
            name.to_owned()
        } else {
            format!("{prefix}/{name}")
        };
        let dir_fd = stream.dir_fd();
        let st = stat_at(dir_fd, OsStr::new(name)).map_err(|_| PreflightError::Io)?;
        if st.is_dir() {
            if stack.len() >= MAX_CANDIDATE_WALK_DEPTH {
                return Err(PreflightError::Oversize);
            }
            let child =
                DirStream::open_at(dir_fd, OsStr::new(name)).map_err(|_| PreflightError::Io)?;
            stack.push((child, relative));
        } else if st.is_symlink() {
            count += 1;
            if count > MAX_SNAPSHOT_FILES {
                return Err(PreflightError::Oversize);
            }
            let target = read_link_at(dir_fd, OsStr::new(name), MAX_SYMLINK_TARGET_BYTES).map_err(
                |error| match error.kind() {
                    ErrorKind::InvalidData => PreflightError::Oversize,
                    _ => PreflightError::Io,
                },
            )?;
            total = total
                .checked_add(target.len() as u64)
                .ok_or(PreflightError::Oversize)?;
            if total > MAX_SNAPSHOT_TOTAL_BYTES {
                return Err(PreflightError::Oversize);
            }
            links.insert(&relative, target);
        }
    }
    links
        .validate()
        .map_err(|refusal| PreflightError::UnsafeSymlink {
            path: refusal.path,
            reason: refusal.reason,
        })
}

/// The candidate symlink walk's depth bound, matching the delta walk's.
#[cfg(unix)]
const MAX_CANDIDATE_WALK_DEPTH: usize = 64;

/// The probe's one gate: cancelled beats expired, and both beat success.
#[cfg(unix)]
fn check_gate(cancellation: &CancellationToken, deadline: Instant) -> Result<(), PreflightError> {
    if cancellation.is_cancelled() {
        return Err(PreflightError::Cancelled);
    }
    if Instant::now() >= deadline {
        return Err(PreflightError::DeadlineExceeded);
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

/// Non-Unix hosts have no anchored probe — no descriptor-relative, no-follow
/// opens to lean on — so every path refuses and the preflight surfaces `Io`.
#[cfg(not(unix))]
pub(super) fn classify_worktree(
    root_fd: RawFd,
    rel: &str,
    baseline: Option<(&Sha256Digest, SnapshotFileMode)>,
    state: &mut ReadState,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<Option<CollisionKind>, PreflightError> {
    let _ = (root_fd, rel, baseline, state, cancellation, deadline);
    Err(PreflightError::Io)
}
