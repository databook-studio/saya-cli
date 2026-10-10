//! Private-directory mechanics for the snapshot artifacts root and its
//! staging directories: exclusive 0700 creation under a pinned descriptor,
//! the umask-forcing re-check, the reopened-inode comparison that keeps
//! commit and cleanup off a swapped name, and the owned-private rule the
//! root is refused against. Supported on macOS and Linux only; every other
//! target has no such code at all. Shared by the destination's root open,
//! its staging creation, and the garbage sweep's name handling.
//!
//! Residual, stated plainly: a same-owner process that can write inside the
//! private 0700 root is inside the trust boundary. The identity checks here
//! are defence against accidents and symlink tricks, not against a hostile
//! same-user process.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::{
    ffi::{CString, OsStr},
    fs, io,
    os::{
        fd::{AsRawFd, RawFd},
        unix::ffi::OsStrExt,
    },
};

use super::{SnapshotError, capture_dest::capture_io};
use crate::workspace::fd::{open_dir_at, stat_at};

/// Owned-private is the sibling git-state rule: group/other bits clear and
/// owned by the effective user. A foreign directory is never chmod-ed into
/// privacy — it is refused outright.
pub(super) fn ensure_private_root(metadata: &fs::Metadata) -> Result<(), SnapshotError> {
    use std::os::unix::fs::MetadataExt as _;
    if metadata.file_type().is_symlink() || !metadata.is_dir() || !owned_private(metadata) {
        return Err(SnapshotError::ArtifactRootRefused);
    }
    Ok(())
}

pub(super) fn owned_private(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    metadata.mode() & 0o077 == 0 && metadata.uid() == unsafe { libc::geteuid() }
}

/// Creates one directory at 0700, exclusively: a pre-existing name of any
/// kind fails, so a staging directory can never be adopted, only created.
pub(super) fn mkdir_private_at(dir_fd: RawFd, name: &OsStr) -> io::Result<()> {
    let c = c_name(name)?;
    let rc = unsafe { libc::mkdirat(dir_fd, c.as_ptr(), 0o700) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Reopens a just-created directory through its pinned parent and verifies
/// the reopened descriptor still names the entry creation made: `mkdirat`
/// returns no descriptor, so the created entry is opened immediately after
/// creation and the descriptor's own `fstat` must equal a fresh
/// `fstatat(AT_SYMLINK_NOFOLLOW)` of the name, refusing a swapped name
/// instead of adopting whatever now holds it.
pub(super) fn reopen_created_private_dir(
    parent: &fs::File,
    name: &OsStr,
    context: &'static str,
) -> Result<fs::File, SnapshotError> {
    use std::os::unix::fs::MetadataExt as _;
    let dir = fs::File::from(open_dir_at(parent.as_raw_fd(), name).map_err(|error| {
        if error.raw_os_error() == Some(libc::ELOOP) {
            SnapshotError::ArtifactRootRefused
        } else {
            capture_io(context, &error)
        }
    })?);
    let opened = dir
        .metadata()
        .map_err(|error| capture_io(context, &error))?;
    let named = stat_at(parent.as_raw_fd(), name).map_err(|error| capture_io(context, &error))?;
    if opened.is_dir() && named.is_dir() && (opened.dev(), opened.ino()) == named.identity() {
        Ok(dir)
    } else {
        Err(SnapshotError::CaptureIo {
            description: format!("keep a created {context} under its name"),
            source: None,
        })
    }
}

/// Re-establishes, immediately before a name-based boundary action (the
/// publish rename in commit and the staging removal in drop), that the
/// name still holds the very directory the retained descriptor was opened
/// on: the descriptor's `fstat` must equal a fresh
/// `fstatat(AT_SYMLINK_NOFOLLOW)` of the name. A mismatch refuses; on the
/// drop path the caller leaves the stranger's entry as sweepable garbage.
pub(super) fn verify_named_identity(
    parent: &fs::File,
    name: &OsStr,
    dir: &fs::File,
    context: &'static str,
) -> Result<(), SnapshotError> {
    use std::os::unix::fs::MetadataExt as _;
    let held = dir
        .metadata()
        .map_err(|error| capture_io(context, &error))?;
    let named = stat_at(parent.as_raw_fd(), name).map_err(|error| capture_io(context, &error))?;
    if held.is_dir() && named.is_dir() && (held.dev(), held.ino()) == named.identity() {
        Ok(())
    } else {
        Err(SnapshotError::CaptureIo {
            description: format!("keep {context} under its own name"),
            source: None,
        })
    }
}

/// The kernel applies the umask to `mkdirat`'s mode, so a just-created
/// directory is forced back to 0700 through its own descriptor only, then
/// re-checked for mode and ownership through that same descriptor.
pub(super) fn privatize_dir(dir: &fs::File, context: &'static str) -> Result<(), SnapshotError> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    dir.set_permissions(fs::Permissions::from_mode(0o700))
        .map_err(|error| capture_io(context, &error))?;
    let metadata = dir
        .metadata()
        .map_err(|error| capture_io(context, &error))?;
    if metadata.mode() & 0o777 != 0o700 || !owned_private(&metadata) {
        return Err(SnapshotError::CaptureIo {
            description: format!("keep a created {context} private"),
            source: None,
        });
    }
    Ok(())
}

/// Opens the final artifacts root as one entry of the pinned parent
/// `parent`, which must already exist. An absent name is created
/// exclusively at 0700 and fchmod-ed only through the descriptor just
/// created; an existing entry is accepted only when it already is an
/// owned-private directory, verified through a no-follow descriptor — an
/// existing root is never chmod-ed.
pub(super) fn open_or_create_private_root(
    parent: &fs::File,
    name: &OsStr,
) -> Result<fs::File, SnapshotError> {
    match mkdir_private_at(parent.as_raw_fd(), name) {
        Ok(()) => {
            let dir = reopen_created_private_dir(parent, name, "artifacts root")?;
            privatize_dir(&dir, "artifacts root")?;
            Ok(dir)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let existing = stat_at(parent.as_raw_fd(), name)
                .map_err(|error| capture_io("scan artifacts root", &error))?;
            if existing.is_symlink() || !existing.is_dir() {
                return Err(SnapshotError::ArtifactRootRefused);
            }
            let dir = fs::File::from(open_dir_at(parent.as_raw_fd(), name).map_err(|error| {
                if error.raw_os_error() == Some(libc::ELOOP) {
                    SnapshotError::ArtifactRootRefused
                } else {
                    capture_io("open artifacts root", &error)
                }
            })?);
            let metadata = dir
                .metadata()
                .map_err(|error| capture_io("scan artifacts root", &error))?;
            ensure_private_root(&metadata)?;
            Ok(dir)
        }
        Err(error) => Err(capture_io("create artifacts root", &error)),
    }
}

pub(super) fn c_name(name: &OsStr) -> io::Result<CString> {
    CString::new(name.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in component"))
}
