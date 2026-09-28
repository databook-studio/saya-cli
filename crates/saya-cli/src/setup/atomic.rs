//! Private-permission file primitives for the setup engine: bounded reads,
//! symlink refusal, private directory creation, and atomic publish via a
//! private temp file + fsync + rename (the pattern used by the TUI history
//! persistence).

use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::Path,
};

use super::SetupError;

fn io_error(path: &Path, source: io::Error) -> SetupError {
    SetupError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn symlink_refused(path: &Path) -> SetupError {
    SetupError::Symlink {
        path: path.to_path_buf(),
    }
}

/// Refuses a path whose final component is a symlink (dangling included).
pub(crate) fn refuse_symlink(path: &Path) -> Result<(), SetupError> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(symlink_refused(path)),
        Ok(_) => Ok(()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error(path, source)),
    }
}

/// Creates `dir` (and parents) when missing, private (0700) when we create it.
/// An existing directory keeps its permissions.
pub(crate) fn ensure_private_dir(dir: &Path) -> Result<(), SetupError> {
    if dir.exists() {
        return Ok(());
    }
    fs::create_dir_all(dir).map_err(|source| io_error(dir, source))?;
    set_private_dir(dir)
}

pub(crate) fn read_bounded(path: &Path, max: u64) -> Result<Vec<u8>, SetupError> {
    let mut file = fs::File::open(path).map_err(|source| io_error(path, source))?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| io_error(path, source))?;
    if bytes.len() as u64 > max {
        return Err(SetupError::TooLarge {
            path: path.to_path_buf(),
            max,
        });
    }
    Ok(bytes)
}

/// `Some(bytes)` when the path exists (never a symlink), `None` when absent.
pub(crate) fn read_optional_bounded(path: &Path, max: u64) -> Result<Option<Vec<u8>>, SetupError> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(symlink_refused(path)),
        Ok(_) => Ok(Some(read_bounded(path, max)?)),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(io_error(path, source)),
    }
}

/// Publishes `bytes` at `path` atomically and privately (0600): writes a
/// hidden sibling temp file, fsyncs it, renames it over `path`. Never follows
/// a symlink at `path` — a rename replaces one, and we refuse first.
pub(crate) fn publish(path: &Path, bytes: &[u8]) -> Result<(), SetupError> {
    refuse_symlink(path)?;
    let name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or_else(|| {
            io_error(
                path,
                io::Error::other("target must be a plain file name in an existing directory"),
            )
        })?;
    let dir = path
        .parent()
        .ok_or_else(|| io_error(path, io::Error::other("target has no parent directory")))?;
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let result = write_staged(&tmp, bytes).and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map_err(|source| io_error(path, source))
}

fn write_staged(tmp: &Path, bytes: &[u8]) -> io::Result<()> {
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    Ok(())
}

/// Removes a file or directory tree if present; refuses a symlink; missing is
/// fine (idempotent).
pub(crate) fn remove_if_present(path: &Path, tree: bool) -> Result<(), SetupError> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(symlink_refused(path)),
        Ok(_) => {
            let removed = if tree {
                fs::remove_dir_all(path)
            } else {
                fs::remove_file(path)
            };
            removed.map_err(|source| io_error(path, source))
        }
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error(path, source)),
    }
}

#[cfg(unix)]
fn set_private_dir(dir: &Path) -> Result<(), SetupError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
        .map_err(|source| io_error(dir, source))
}

#[cfg(not(unix))]
fn set_private_dir(_dir: &Path) -> Result<(), SetupError> {
    Ok(())
}
