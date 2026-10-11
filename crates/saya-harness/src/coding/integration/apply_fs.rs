//! The child-file seam for applying an accepted delta: the content read and
//! the mode stat for one child file of the verified checkout.
//!
//! All accesses are descriptor-relative and no-follow: every access starts
//! from the descriptor each workspace pinned at open, so a replaced directory
//! cannot redirect the chmod or the stat. The mode stat and the content read
//! are two opens of the same name under the SAME pinned root: a swap in
//! between can only change the candidate's executable bit, never its bytes,
//! which are digest-verified.

use super::ApplyError;

use crate::workspace::Workspace;

#[cfg(unix)]
use crate::workspace::MAX_IO_BYTES;
#[cfg(unix)]
use crate::workspace::fd::{open_dir_at, open_file_at, stat_at};
#[cfg(unix)]
use std::ffi::OsStr;
#[cfg(unix)]
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// One child file re-read at apply time: the bytes, their sha256 hex digest,
/// and the executable bit observed by the anchored stat.
pub(super) struct ChildFile {
    pub(super) bytes: Vec<u8>,
    pub(super) sha256_hex: String,
    pub(super) executable: bool,
}

/// Re-reads one child file no-follow from the already-open child workspace:
/// the content through the workspace reader, then the executable bit through
/// an anchored `fstatat` walk of the same name, both under the root
/// descriptor the workspace pinned at open.
#[cfg(unix)]
pub(super) fn read_child_file(workspace: &Workspace, rel: &str) -> Result<ChildFile, ApplyError> {
    let root_fd = workspace.root_dir_fd().as_raw_fd();
    let read = workspace
        .read(rel, MAX_IO_BYTES as u64)
        .map_err(|error| match error {
            crate::HarnessError::BoundsExceeded { .. } => ApplyError::Oversize,
            _ => ApplyError::Fs,
        })?;
    if read.truncated {
        return Err(ApplyError::Oversize);
    }
    let (dir, last) = open_parent(root_fd, rel)?;
    let stat = stat_at(dir.as_raw_fd(), OsStr::new(last)).map_err(|_| ApplyError::Fs)?;
    if !stat.is_file() {
        return Err(ApplyError::Fs);
    }
    Ok(ChildFile {
        bytes: read.bytes,
        sha256_hex: read.digest,
        executable: stat.has_exec_bits(),
    })
}

/// Fixes the final mode of the just-written candidate file by fchmod on a
/// no-follow open of the same name from the already-open candidate
/// workspace's pinned root descriptor: the Workspace writes files 0600, so
/// this sets the mode the receipt recorded.
#[cfg(unix)]
pub(super) fn set_candidate_mode(
    workspace: &Workspace,
    rel: &str,
    executable: bool,
) -> Result<(), ApplyError> {
    let (parent, last) = open_parent(workspace.root_dir_fd().as_raw_fd(), rel)?;
    let file = open_file_at(parent.as_raw_fd(), OsStr::new(last), libc::O_RDONLY, 0)
        .map_err(|_| ApplyError::Fs)?;
    let mode = if executable { 0o755 } else { 0o644 };
    file.set_permissions(std::fs::Permissions::from_mode(mode))
        .map_err(|_| ApplyError::Fs)
}

/// Walks `rel`'s directory components no-follow from the pinned root
/// descriptor and returns the parent directory descriptor plus the final
/// component name. The root descriptor is only borrowed: the returned
/// descriptor is always a fresh open, never the root itself.
#[cfg(unix)]
fn open_parent<'a>(root_fd: RawFd, rel: &'a str) -> Result<(OwnedFd, &'a str), ApplyError> {
    let mut components = rel.split('/');
    let last = match components.next_back() {
        Some(last) if !last.is_empty() => last,
        _ => return Err(ApplyError::Fs),
    };
    let mut cur: Option<OwnedFd> = None;
    for component in components {
        if component.is_empty() {
            return Err(ApplyError::Fs);
        }
        cur = Some(
            open_dir_at(
                cur.as_ref().map_or(root_fd, |fd| fd.as_raw_fd()),
                OsStr::new(component),
            )
            .map_err(|_| ApplyError::Fs)?,
        );
    }
    let parent = match cur {
        Some(dir) => dir,
        None => open_dir_at(root_fd, OsStr::new(".")).map_err(|_| ApplyError::Fs)?,
    };
    Ok((parent, last))
}

/// Make bytes, mode, and all directory entries durable before journal completion.
#[cfg(unix)]
pub(super) fn sync_applied_entry(
    workspace: &Workspace,
    rel: &str,
    deleted: bool,
) -> Result<(), ApplyError> {
    let mut parts = rel.split('/');
    let last = parts.next_back().ok_or(ApplyError::Fs)?;
    let root = open_dir_at(workspace.root_dir_fd().as_raw_fd(), OsStr::new("."))
        .map_err(|_| ApplyError::Fs)?;
    let mut parents = vec![root];
    for part in parts {
        let parent = parents.last().ok_or(ApplyError::Fs)?;
        let next = open_dir_at(parent.as_raw_fd(), OsStr::new(part)).map_err(|_| ApplyError::Fs)?;
        parents.push(next);
    }
    if !deleted {
        let parent = parents.last().ok_or(ApplyError::Fs)?;
        let file = open_file_at(
            parent.as_raw_fd(),
            OsStr::new(last),
            libc::O_RDONLY | libc::O_NONBLOCK,
            0,
        )
        .map_err(|_| ApplyError::Fs)?;
        if !file.metadata().map_err(|_| ApplyError::Fs)?.is_file() {
            return Err(ApplyError::Fs);
        }
        file.sync_all().map_err(|_| ApplyError::Fs)?;
    }
    for parent in parents.into_iter().rev() {
        std::fs::File::from(parent)
            .sync_all()
            .map_err(|_| ApplyError::Fs)?;
    }
    Ok(())
}

#[cfg(not(unix))]
pub(super) fn sync_applied_entry(
    _workspace: &Workspace,
    _rel: &str,
    _deleted: bool,
) -> Result<(), ApplyError> {
    Err(ApplyError::Fs)
}

#[cfg(not(unix))]
pub(super) fn read_child_file(_workspace: &Workspace, _rel: &str) -> Result<ChildFile, ApplyError> {
    Err(ApplyError::Fs)
}

#[cfg(not(unix))]
pub(super) fn set_candidate_mode(
    _workspace: &Workspace,
    _rel: &str,
    _executable: bool,
) -> Result<(), ApplyError> {
    Err(ApplyError::Fs)
}
