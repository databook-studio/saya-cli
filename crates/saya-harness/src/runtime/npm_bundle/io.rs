//! No-follow owner-only file operations for the input bundle.

use std::{fs, io::Read as _, path::Path};

use super::super::npm::NpmRuntimeError;

pub(super) fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), NpmRuntimeError> {
    use std::io::Write as _;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let mut file = options.open(path).map_err(|_| NpmRuntimeError::Failed)?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| NpmRuntimeError::Failed)
}

pub(super) fn create_private_dir(path: &Path) -> Result<(), NpmRuntimeError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        let mut builder = fs::DirBuilder::new();
        builder
            .mode(0o700)
            .create(path)
            .map_err(|_| NpmRuntimeError::Failed)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(NpmRuntimeError::Failed)
    }
}

pub(super) fn ensure_private_dir(path: &Path) -> Result<(), NpmRuntimeError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| NpmRuntimeError::StateConflict)?;
    if is_private_dir(&metadata) {
        Ok(())
    } else {
        Err(NpmRuntimeError::StateConflict)
    }
}

pub(super) fn read_private_file(path: &Path, limit: u64) -> Result<Vec<u8>, NpmRuntimeError> {
    let file = open_private_file(path, limit)?;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| NpmRuntimeError::StateConflict)?;
    if bytes.len() as u64 > limit {
        return Err(NpmRuntimeError::StateConflict);
    }
    Ok(bytes)
}

pub(super) fn open_private_file(path: &Path, limit: u64) -> Result<fs::File, NpmRuntimeError> {
    let pre = fs::symlink_metadata(path).map_err(|_| NpmRuntimeError::StateConflict)?;
    if !is_private_file(&pre) || pre.len() > limit {
        return Err(NpmRuntimeError::StateConflict);
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    let file = options
        .open(path)
        .map_err(|_| NpmRuntimeError::StateConflict)?;
    let opened = file
        .metadata()
        .map_err(|_| NpmRuntimeError::StateConflict)?;
    if !is_private_file(&opened) || opened.len() != pre.len() || !same_file(&pre, &opened) {
        return Err(NpmRuntimeError::StateConflict);
    }
    Ok(file)
}

pub(super) fn is_private_file(metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        metadata.is_file()
            && !metadata.file_type().is_symlink()
            && metadata.mode() & 0o777 == 0o600
            && metadata.nlink() == 1
            && metadata.uid() == unsafe { libc::geteuid() }
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        false
    }
}

pub(super) fn is_private_dir(metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && metadata.mode() & 0o777 == 0o700
            && metadata.uid() == unsafe { libc::geteuid() }
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        false
    }
}

pub(super) fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        left.dev() == right.dev() && left.ino() == right.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = (left, right);
        false
    }
}

pub(super) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}
