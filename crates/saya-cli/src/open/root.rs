//! Where staged file sources live and how private directories are made:
//! the files-root resolution (`SAYA_FILES_DIR` else `<data root>/saya/files`),
//! the root's creation at 0700, and the guarded private staging directory the
//! one-time read lands in before its snapshot is placed.

use std::{
    fs, io,
    path::{Path, PathBuf},
    process,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// Where staged file sources live: `SAYA_FILES_DIR` when set, else
/// `<data root>/saya/files` beside the state database. Always absolute, so a
/// generated connections file keeps working when a session launches later.
pub(super) fn files_root() -> PathBuf {
    let candidate = match std::env::var_os("SAYA_FILES_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => crate::state_path::state_db_path()
            .parent()
            .map(|parent| parent.join("files"))
            .unwrap_or_else(|| PathBuf::from("saya/files")),
    };
    absolute(&candidate)
}

fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

/// Creates the files root (0700, never a symlink) when missing.
pub(super) fn ensure_root(root: &Path) -> Result<(), String> {
    fs::create_dir_all(root)
        .map_err(|error| format!("create the files root {}: {error}", root.display()))?;
    let meta =
        fs::symlink_metadata(root).map_err(|error| format!("stat {}: {error}", root.display()))?;
    if meta.is_symlink() || !meta.is_dir() {
        return Err(format!("{} is not a real directory", root.display()));
    }
    #[cfg(unix)]
    fs::set_permissions(root, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("restrict {} to 0700: {error}", root.display()))?;
    Ok(())
}

fn nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

/// A fresh private staging directory under the files root, at 0700.
pub(super) fn fresh_temp_dir(root: &Path) -> Result<PathBuf, String> {
    for _ in 0..8 {
        let candidate = root.join(format!(".staging-{}-{}", process::id(), nanos()));
        match fs::create_dir(&candidate) {
            Ok(()) => {
                #[cfg(unix)]
                fs::set_permissions(&candidate, fs::Permissions::from_mode(0o700)).map_err(
                    |error| format!("restrict {} to 0700: {error}", candidate.display()),
                )?;
                return Ok(candidate);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("create staging directory: {error}")),
        }
    }
    Err("could not create a private staging directory under the files root".into())
}

/// Guards the private staging directory: while armed, dropping removes it, so
/// every failure path — including panics — leaves no staging directory behind.
pub(super) struct TempDirGuard {
    path: PathBuf,
    armed: bool,
}

impl TempDirGuard {
    pub(super) fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }
    pub(super) fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
