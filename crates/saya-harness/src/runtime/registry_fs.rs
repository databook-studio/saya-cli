//! No-follow private file and directory operations for the runtime registry.

#[path = "registry_state_root.rs"]
mod state_root;

#[cfg(unix)]
use std::fs::DirBuilder;
#[cfg(unix)]
use std::fs::OpenOptions;
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _};
use std::{
    fs::{self, File},
    io,
    path::{Path, PathBuf},
};

#[cfg(unix)]
const NO_FOLLOW: i32 = libc::O_NOFOLLOW | libc::O_CLOEXEC;

pub(super) fn ensure_state_root(path: &Path) -> Result<PathBuf, super::RegistryError> {
    state_root::ensure(path)
}

pub(super) fn existing_state_root(path: &Path) -> Result<Option<PathBuf>, super::RegistryError> {
    state_root::existing(path)
}

pub(super) fn create_private_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        let mut builder = DirBuilder::new();
        builder.mode(0o700).create(path)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(io::Error::other("private state is unsupported"))
    }
}

pub(super) fn check_private_dir(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if private_dir(&metadata) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsafe directory",
        ))
    }
}

pub(super) fn create_private_file(path: &Path) -> io::Result<File> {
    #[cfg(unix)]
    {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(NO_FOLLOW)
            .open(path)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(io::Error::other("private state is unsupported"))
    }
}

pub(super) fn open_read(path: &Path) -> io::Result<File> {
    #[cfg(unix)]
    {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(NO_FOLLOW | libc::O_NONBLOCK)
            .open(path)?;
        check_private_file_metadata(&file.metadata()?)?;
        Ok(file)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(io::Error::other("private state is unsupported"))
    }
}

pub(super) fn open_lock(path: &Path) -> io::Result<File> {
    #[cfg(unix)]
    {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(NO_FOLLOW | libc::O_NONBLOCK)
            .open(path)?;
        check_private_file_metadata(&file.metadata()?)?;
        file.sync_all()?;
        Ok(file)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(io::Error::other("private state is unsupported"))
    }
}

pub(super) fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        OpenOptions::new()
            .read(true)
            .custom_flags(NO_FOLLOW | libc::O_DIRECTORY)
            .open(path)?
            .sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(io::Error::other("private state is unsupported"))
    }
}

#[cfg(unix)]
pub(super) fn check_private_stage(path: &Path, ready: &Path) -> io::Result<()> {
    let staged = fs::symlink_metadata(path)?;
    if !private_file_owner_mode(&staged) || staged.len() > 512 || staged.nlink() > 2 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "unsafe stage"));
    }
    if staged.nlink() == 2 {
        let ready = fs::symlink_metadata(ready)?;
        if !private_file_owner_mode(&ready)
            || ready.nlink() != 2
            || ready.dev() != staged.dev()
            || ready.ino() != staged.ino()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unowned hard link",
            ));
        }
    }
    Ok(())
}
#[cfg(not(unix))]
pub(super) fn check_private_stage(_: &Path, _: &Path) -> io::Result<()> {
    Err(io::Error::other("private state is unsupported"))
}

#[cfg(unix)]
fn check_private_file_metadata(metadata: &fs::Metadata) -> io::Result<()> {
    if private_file_metadata(metadata) {
        return Ok(());
    }
    Err(io::Error::new(io::ErrorKind::InvalidData, "unsafe file"))
}

#[cfg(unix)]
fn private_dir(metadata: &fs::Metadata) -> bool {
    metadata.is_dir()
        && !metadata.file_type().is_symlink()
        && metadata.mode() & 0o777 == 0o700
        && metadata.uid() == unsafe { libc::geteuid() }
}
#[cfg(not(unix))]
fn private_dir(_: &fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
fn private_file_metadata(metadata: &fs::Metadata) -> bool {
    private_file_owner_mode(metadata) && metadata.nlink() == 1
}
#[cfg(unix)]
fn private_file_owner_mode(metadata: &fs::Metadata) -> bool {
    metadata.is_file()
        && !metadata.file_type().is_symlink()
        && metadata.mode() & 0o777 == 0o600
        && metadata.uid() == unsafe { libc::geteuid() }
}
