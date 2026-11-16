//! The Windows Git service state root: the same private layout as the unix
//! implementation, with reparse-point refusal standing in for POSIX modes.
//! Windows has no owner/mode bits to check, so the guarantee is structural:
//! the root is an absolute, canonical directory that is not a reparse point,
//! and every directory and file the service creates is checked the same way.

use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use crate::workspace::windows_identity;

use super::super::command::GitError;
use super::GitEnvironment;

impl GitEnvironment {
    pub(in crate::coding::git) fn new(
        executable: PathBuf,
        state_root: &Path,
    ) -> Result<Self, GitError> {
        if !state_root.is_absolute() {
            return Err(GitError::InvalidStateRoot);
        }
        let input_metadata =
            fs::symlink_metadata(state_root).map_err(|_| GitError::InvalidStateRoot)?;
        if input_metadata.file_type().is_symlink() {
            return Err(GitError::InvalidStateRoot);
        }
        let root = fs::canonicalize(state_root).map_err(|_| GitError::InvalidStateRoot)?;
        let metadata = fs::symlink_metadata(&root).map_err(|_| GitError::InvalidStateRoot)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() || root.to_str().is_none() {
            return Err(GitError::InvalidStateRoot);
        }
        if windows_identity::path_is_reparse_point(&root).map_err(|_| GitError::InvalidStateRoot)? {
            return Err(GitError::InvalidStateRoot);
        }
        let root = simplify(root);
        let worktrees = root.join("worktrees");
        let hooks = root.join("empty-hooks");
        let temporary = root.join("tmp");
        let empty_global_config = root.join("empty.gitconfig");
        ensure_private_dir(&hooks)?;
        ensure_empty_dir(&hooks)?;
        ensure_private_dir(&worktrees)?;
        ensure_private_dir(&temporary)?;
        ensure_empty_file(&empty_global_config)?;
        Ok(Self {
            root,
            worktrees,
            hooks,
            temporary,
            empty_global_config,
            executable,
            compatibility: std::sync::OnceLock::new(),
        })
    }
}

fn ensure_private_dir(path: &Path) -> Result<(), GitError> {
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(GitError::InvalidStateRoot),
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| GitError::InvalidStateRoot)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || windows_identity::path_is_reparse_point(path).map_err(|_| GitError::InvalidStateRoot)?
    {
        return Err(GitError::InvalidStateRoot);
    }
    Ok(())
}

fn ensure_empty_dir(path: &Path) -> Result<(), GitError> {
    let mut entries = fs::read_dir(path).map_err(|_| GitError::InvalidStateRoot)?;
    match entries.next() {
        None => Ok(()),
        Some(Ok(_)) | Some(Err(_)) => Err(GitError::InvalidStateRoot),
    }
}

fn ensure_empty_file(path: &Path) -> Result<(), GitError> {
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(GitError::InvalidStateRoot),
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| GitError::InvalidStateRoot)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() != 0
        || windows_identity::path_is_reparse_point(path).map_err(|_| GitError::InvalidStateRoot)?
    {
        return Err(GitError::InvalidStateRoot);
    }
    Ok(())
}

/// `canonicalize` returns a `\\?\` verbatim path; Git for Windows accepts
/// them, but the environment values the service hands to Git are ordinary
/// paths. Strip the prefix when the remainder is already absolute.
fn simplify(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path;
    };
    let Some(rest) = text.strip_prefix(r"\\?\") else {
        return path;
    };
    match rest.strip_prefix(r"UNC\") {
        Some(unc) => PathBuf::from(format!(r"\\{unc}")),
        None => PathBuf::from(rest),
    }
}
