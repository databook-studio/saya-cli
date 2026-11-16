//! The Windows child-file seam for applying an accepted delta. Windows has
//! no descriptor-relative `openat` and no executable bit: every access is
//! the contained workspace walk, which refuses reparse points and verifies
//! the opened handle's identity, and the mode is the fixed regular-file
//! mode. Content is still re-read and digest-checked at apply time.

use std::{fs, os::windows::fs::OpenOptionsExt as _};

use crate::workspace::{
    MAX_IO_BYTES, Workspace, contain::FILE_FLAG_OPEN_REPARSE_POINT, windows_identity,
};

use super::{ApplyError, ChildFile};

/// The fixed mode Windows records for a regular file: no execute bit.
const REGULAR_MODE: u32 = 0o644;

/// Re-reads one child file through the contained workspace: the same
/// no-follow open and post-open identity check every Windows read uses.
pub(crate) fn read_child_file(workspace: &Workspace, rel: &str) -> Result<ChildFile, ApplyError> {
    let read = workspace
        .read(rel, MAX_IO_BYTES as u64)
        .map_err(|error| match error {
            crate::HarnessError::BoundsExceeded { .. } => ApplyError::Oversize,
            _ => ApplyError::Fs,
        })?;
    if read.truncated {
        return Err(ApplyError::Oversize);
    }
    Ok(ChildFile {
        bytes: read.bytes,
        sha256_hex: read.digest,
        executable: false,
    })
}

/// Windows has no mode bits to set: the candidate file was just published
/// by the contained write, so only the mode shape is checked.
pub(crate) fn set_candidate_mode_bits(
    workspace: &Workspace,
    rel: &str,
    mode: u32,
) -> Result<(), ApplyError> {
    if mode > 0o777 {
        return Err(ApplyError::Fs);
    }
    require_plain_file(workspace, rel)
}

/// Syncs the applied file's bytes. Windows cannot open a directory for
/// `sync_all`, so the directory half is a no-op there.
pub(crate) fn sync_applied_entry(
    workspace: &Workspace,
    rel: &str,
    deleted: bool,
) -> Result<(), ApplyError> {
    if deleted {
        return Ok(());
    }
    // The verification open is read-only, and `FlushFileBuffers` refuses a
    // handle without write access. Flush through a second open of the same
    // path, which the workspace walk re-checks and which refuses a reparse
    // point like every other write in this seam.
    drop(open_plain_file(workspace, rel)?);
    let path = workspace.target(rel, false).map_err(|_| ApplyError::Fs)?;
    let file = fs::OpenOptions::new()
        .write(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&path)
        .map_err(|_| ApplyError::Fs)?;
    file.sync_all().map_err(|_| ApplyError::Fs)
}

/// The mode observed for a regular file. Windows snapshots record one fixed
/// regular mode; anything that is not a plain regular file refuses.
pub(crate) fn file_mode(workspace: &Workspace, rel: &str) -> Result<u32, ApplyError> {
    require_plain_file(workspace, rel)?;
    Ok(REGULAR_MODE)
}

fn require_plain_file(workspace: &Workspace, rel: &str) -> Result<(), ApplyError> {
    open_plain_file(workspace, rel).map(|_| ())
}

/// Opens the entry as the link itself and refuses a reparse point, a
/// symlink, or a non-regular file. The workspace walk already validates
/// every component; the opened handle's identity is checked again.
fn open_plain_file(workspace: &Workspace, rel: &str) -> Result<fs::File, ApplyError> {
    let path = workspace.target(rel, false).map_err(|_| ApplyError::Fs)?;
    let pre = fs::symlink_metadata(&path).map_err(|_| ApplyError::Fs)?;
    if pre.file_type().is_symlink() || !pre.is_file() {
        return Err(ApplyError::Fs);
    }
    if windows_identity::path_is_reparse_point(&path).map_err(|_| ApplyError::Fs)? {
        return Err(ApplyError::Fs);
    }
    let file = workspace
        .open_verified(&path, &pre, rel)
        .map_err(|_| ApplyError::Fs)?;
    if !file.metadata().map_err(|_| ApplyError::Fs)?.is_file() {
        return Err(ApplyError::Fs);
    }
    Ok(file)
}
