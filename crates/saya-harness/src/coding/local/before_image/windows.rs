//! The Windows no-follow opens and private directories of the before-image
//! store. Windows has no `O_NOFOLLOW`, so every open asks for the link itself
//! (`FILE_FLAG_OPEN_REPARSE_POINT`) and the opened handle is refused when it
//! names a reparse point. The directory check can prove a real directory that
//! is not a link; Windows privacy rests on the profile ACL that owns the
//! state root, so no mode bits are asserted here.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek as _, SeekFrom};
use std::os::windows::fs::OpenOptionsExt as _;
use std::path::Path;

use crate::workspace::windows_identity;

const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

/// Opens an existing object for reading as the link itself and refuses a
/// reparse point or a non-regular file.
pub(super) fn open_read(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    require_plain_file(&file)?;
    Ok(file)
}

/// Opens the index for appending, creating it when absent, and refuses a
/// reparse point or a non-regular file.
pub(super) fn open_append(path: &Path) -> io::Result<File> {
    // `FILE_APPEND_DATA` alone cannot be flushed (`FlushFileBuffers` needs
    // `FILE_WRITE_DATA`), so ask for ordinary write access and seek to the
    // end before each append.
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    require_plain_file(&file)?;
    file.seek(SeekFrom::End(0))?;
    Ok(file)
}

/// Creates a new file and refuses an existing reparse point in its place.
pub(super) fn open_create_new(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    require_plain_file(&file)?;
    Ok(file)
}

/// Creates the store directory when absent and refuses one that is a reparse
/// point or not a directory.
pub(super) fn private_directory(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || windows_identity::path_is_reparse_point(path)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "before-image directory is not a real directory",
        ));
    }
    Ok(())
}

fn require_plain_file(file: &File) -> io::Result<()> {
    if windows_identity::is_reparse_point(file)? || !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "before-image object is not a regular file",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    /// Real Windows refuses `FlushFileBuffers` on an append-only handle
    /// (`FILE_APPEND_DATA` without `FILE_WRITE_DATA`). An append handle also
    /// ignores the write position, so a write at offset zero proves the
    /// returned handle carries ordinary write access.
    #[test]
    fn append_handle_flushes_and_writes_at_the_seek_position() {
        let path = std::env::temp_dir().join(format!(
            "saya-before-image-append-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut file = open_append(&path).unwrap();
        file.write_all(b"first\n").unwrap();
        file.sync_all().unwrap();
        file.seek(SeekFrom::End(0)).unwrap();
        file.write_all(b"second\n").unwrap();
        file.sync_all().unwrap();
        drop(file);
        assert_eq!(fs::read(&path).unwrap(), b"first\nsecond\n");
        let mut file = open_append(&path).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(b"X").unwrap();
        file.sync_all().unwrap();
        drop(file);
        assert_eq!(fs::read(&path).unwrap(), b"Xirst\nsecond\n");
        let _ = fs::remove_file(&path);
    }
}
