//! The one private directory a host creates under its state root, and the
//! filesystem error that names the path it failed on.
//!
//! Job records, snapshots, rewind stores and in-place leases all live under
//! directories this creates: owner-only, canonical, and never altered once it
//! exists. It lives beside the host it serves rather than in it, because every
//! one of those readers asks for the same rule.

use super::CodingError;
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Creates `path` (and any missing ancestor) owner-private, and returns its
/// canonical form. It refuses a leaf that is not a directory, is reachable by
/// anyone else, or is owned by another user: a state directory another account
/// can read is no place for a job's records.
pub(crate) fn private_directory(path: &Path) -> Result<PathBuf, CodingError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt};
        // Missing ancestors are created too (private, like the leaf): on a
        // fresh machine saya's own state directory does not exist yet. An
        // existing directory is never altered; only the leaf is then checked.
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        match builder.create(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(CodingError::owner(error)),
        }
        let metadata = fs::symlink_metadata(path).map_err(CodingError::owner)?;
        // SAFETY: geteuid reads the process identity and has no pointer arguments.
        let owner_uid = unsafe { libc::geteuid() };
        if !metadata.is_dir() || metadata.mode() & 0o077 != 0 || metadata.uid() != owner_uid {
            return Err(CodingError::StateDirectory);
        }
        fs::canonicalize(path).map_err(CodingError::owner)
    }
    #[cfg(windows)]
    {
        // Windows has no owner-only mode bits: the state directory is
        // created under the user's profile, whose ACL is the privacy
        // boundary. What can be proved here is that the leaf is a real
        // directory and not a link or junction that would redirect job
        // records elsewhere.
        fs::create_dir_all(path).map_err(CodingError::owner)?;
        let metadata = fs::symlink_metadata(path).map_err(CodingError::owner)?;
        if !metadata.is_dir()
            || saya_harness::workspace::windows_identity::path_is_reparse_point(path)
                .map_err(CodingError::owner)?
        {
            return Err(CodingError::StateDirectory);
        }
        fs::canonicalize(path).map_err(CodingError::owner)
    }
    #[cfg(all(not(unix), not(windows)))]
    {
        let _ = path;
        Err(CodingError::StateDirectory)
    }
}

/// Names the operation and the path in a filesystem error. The OS message
/// alone ("The system cannot find the path specified") does not say which
/// path a launch failed to resolve.
pub(crate) fn io_context(operation: &str, path: &Path, error: std::io::Error) -> std::io::Error {
    std::io::Error::new(
        error.kind(),
        format!("{operation} {}: {error}", path.display()),
    )
}
