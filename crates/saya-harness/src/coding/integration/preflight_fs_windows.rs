//! The Windows read-only probe behind the apply-back preflight. There is no
//! `openat` here: each path goes through the contained workspace walk, which
//! refuses reparse points and verifies the opened handle's identity, and the
//! candidate symlink set is read without following any link.

use std::{
    fs,
    io::{ErrorKind, Read},
    path::Path,
    time::Instant,
};

use saya_agent::CancellationToken;
use saya_types::Sha256Digest;
use sha2::{Digest, Sha256};

use crate::coding::snapshot::{
    MAX_SNAPSHOT_FILE_BYTES, MAX_SNAPSHOT_FILES, MAX_SNAPSHOT_TOTAL_BYTES,
    MAX_SYMLINK_TARGET_BYTES, SnapshotFileMode, SymlinkSet,
};
use crate::workspace::{Workspace, windows_identity};

use super::super::preflight::{CollisionKind, PreflightError};
use super::ReadState;

/// The candidate walk's depth bound, matching the delta walk's.
const MAX_CANDIDATE_WALK_DEPTH: usize = 64;

/// Classifies one accepted delta path against the baseline entry it had at
/// capture — `(content_sha256, mode)` when the path existed, `None` when it
/// did not. Missing where the baseline has an entry is `UserDeleted`, present
/// where it has none is `UserAddedUntracked`, a reparse point or non-regular
/// file is `TypeChanged`, and content, mode, or symlink-target drift is
/// `UserEdited`. Bytes join `state.total_bytes` under the size bounds, the
/// gate is re-checked before every read, and the tree is never written.
pub(crate) fn classify_worktree(
    workspace: &Workspace,
    rel: &str,
    baseline: Option<(&Sha256Digest, SnapshotFileMode)>,
    state: &mut ReadState,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<Option<CollisionKind>, PreflightError> {
    check_gate(cancellation, deadline)?;
    let path = match workspace.target(rel, false) {
        Ok(path) => path,
        Err(crate::HarnessError::NotFound { .. }) => return Ok(deleted_or_none(baseline)),
        Err(crate::HarnessError::SymlinkRefused { .. }) => {
            return classify_symlink_entry(workspace, rel, baseline, state, cancellation, deadline);
        }
        Err(crate::HarnessError::InvalidPath { .. })
        | Err(crate::HarnessError::PathOutsideRoot { .. }) => {
            return Ok(Some(CollisionKind::TypeChanged));
        }
        Err(_) => return Err(PreflightError::Io),
    };
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(deleted_or_none(baseline));
        }
        Err(_) => return Err(PreflightError::Io),
    };
    if meta.file_type().is_symlink() {
        return classify_symlink_entry(workspace, rel, baseline, state, cancellation, deadline);
    }
    if windows_identity::path_is_reparse_point(&path).map_err(|_| PreflightError::Io)? {
        return Ok(Some(CollisionKind::TypeChanged));
    }
    let Some((baseline_digest, baseline_mode)) = baseline else {
        return Ok(Some(if meta.is_file() {
            CollisionKind::UserAddedUntracked
        } else {
            CollisionKind::TypeChanged
        }));
    };
    if !meta.is_file() || baseline_mode == SnapshotFileMode::Symlink {
        return Ok(Some(CollisionKind::TypeChanged));
    }
    if meta.len() > MAX_SNAPSHOT_FILE_BYTES {
        return Err(PreflightError::Oversize);
    }
    let mode = SnapshotFileMode::Regular;
    let mut file = workspace
        .open_verified(&path, &meta, rel)
        .map_err(|_| PreflightError::Io)?;
    let digest = hash_stream(&mut file, state, cancellation, deadline)?;
    if digest != *baseline_digest || mode != baseline_mode {
        return Ok(Some(CollisionKind::UserEdited));
    }
    Ok(None)
}

/// Classifies a final entry that is a symlink (or an intermediate link the
/// workspace walk refused). The parent is re-validated first so a link in an
/// ancestor cannot redirect the read; the link itself is never followed.
fn classify_symlink_entry(
    workspace: &Workspace,
    rel: &str,
    baseline: Option<(&Sha256Digest, SnapshotFileMode)>,
    state: &mut ReadState,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<Option<CollisionKind>, PreflightError> {
    check_gate(cancellation, deadline)?;
    let parent = rel.rsplit_once('/').map(|(parent, _)| parent);
    if let Some(parent) = parent {
        workspace
            .target(parent, false)
            .map_err(|_| PreflightError::Io)?;
    }
    let path = workspace.root().join(rel);
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(deleted_or_none(baseline));
        }
        Err(_) => return Err(PreflightError::Io),
    };
    if !meta.file_type().is_symlink() {
        return Ok(Some(CollisionKind::TypeChanged));
    }
    let Some((baseline_digest, baseline_mode)) = baseline else {
        return Ok(Some(CollisionKind::UserAddedUntracked));
    };
    if baseline_mode != SnapshotFileMode::Symlink {
        return Ok(Some(CollisionKind::TypeChanged));
    }
    let target = fs::read_link(&path).map_err(|_| PreflightError::Io)?;
    let target = target.to_string_lossy().replace('\\', "/").into_bytes();
    if target.len() > MAX_SYMLINK_TARGET_BYTES {
        return Err(PreflightError::Oversize);
    }
    state.total_bytes = state
        .total_bytes
        .checked_add(target.len() as u64)
        .ok_or(PreflightError::Oversize)?;
    if state.total_bytes > MAX_SNAPSHOT_TOTAL_BYTES {
        return Err(PreflightError::Oversize);
    }
    let digest = Sha256Digest::parse(&hex_lower(&Sha256::digest(&target)))
        .map_err(|_| PreflightError::Io)?;
    Ok((digest != *baseline_digest).then_some(CollisionKind::UserEdited))
}

fn deleted_or_none(baseline: Option<(&Sha256Digest, SnapshotFileMode)>) -> Option<CollisionKind> {
    baseline.map(|_| CollisionKind::UserDeleted)
}

/// Streams one regular file under the snapshot file bound while hashing it,
/// joining every byte into the run's total.
fn hash_stream(
    file: &mut fs::File,
    state: &mut ReadState,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<Sha256Digest, PreflightError> {
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
    Sha256Digest::parse(&hex_lower(&hasher.finalize())).map_err(|_| PreflightError::Io)
}

/// Collects every symlink in the job's candidate checkout and validates the
/// whole set chain-aware. The walk reads directories through the contained
/// workspace, skips `.git` and the `.saya` output root, refuses reparse
/// points it cannot read as links, and applies the same depth, count, and
/// byte bounds.
pub(crate) fn validate_candidate_symlinks(
    root: &Path,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<(), PreflightError> {
    let workspace = Workspace::open(root).map_err(|_| PreflightError::Io)?;
    let mut stack: Vec<(String, fs::ReadDir)> = vec![(
        String::new(),
        fs::read_dir(workspace.root()).map_err(|_| PreflightError::Io)?,
    )];
    let mut links = SymlinkSet::default();
    let mut count = 0usize;
    let mut total = 0u64;
    while let Some((prefix, stream)) = stack.last_mut() {
        check_gate(cancellation, deadline)?;
        let Some(entry) = stream.next() else {
            stack.pop();
            continue;
        };
        let entry = entry.map_err(|_| PreflightError::Io)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| PreflightError::Io)?;
        let is_root = prefix.is_empty();
        if name.eq_ignore_ascii_case(".git")
            || (is_root && name == super::super::delta_output::OUTPUT_ROOT_NAME)
        {
            continue;
        }
        let relative = if is_root {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let path = entry.path();
        let file_type = entry.file_type().map_err(|_| PreflightError::Io)?;
        let reparse =
            windows_identity::path_is_reparse_point(&path).map_err(|_| PreflightError::Io)?;
        if file_type.is_dir() && !reparse {
            if stack.len() >= MAX_CANDIDATE_WALK_DEPTH {
                return Err(PreflightError::Oversize);
            }
            let child = fs::read_dir(&path).map_err(|_| PreflightError::Io)?;
            stack.push((relative, child));
        } else if file_type.is_symlink() || reparse {
            count += 1;
            if count > MAX_SNAPSHOT_FILES {
                return Err(PreflightError::Oversize);
            }
            let target = fs::read_link(&path).map_err(|_| PreflightError::UnsafeSymlink {
                path: relative.clone(),
                reason: "the reparse point is not a supported symlink",
            })?;
            let target = target.to_string_lossy().replace('\\', "/").into_bytes();
            if target.len() > MAX_SYMLINK_TARGET_BYTES {
                return Err(PreflightError::Oversize);
            }
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

fn check_gate(cancellation: &CancellationToken, deadline: Instant) -> Result<(), PreflightError> {
    if cancellation.is_cancelled() {
        return Err(PreflightError::Cancelled);
    }
    if Instant::now() >= deadline {
        return Err(PreflightError::DeadlineExceeded);
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
