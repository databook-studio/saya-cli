//! Stable Windows file identity from an open handle.

use std::{
    fs::{File, OpenOptions},
    io,
    os::windows::{fs::OpenOptionsExt as _, io::AsRawHandle},
    path::{Component, Path},
};

#[repr(C)]
struct FileTime {
    _low: u32,
    _high: u32,
}

#[repr(C)]
struct ByHandleFileInformation {
    attributes: u32,
    _creation_time: FileTime,
    _last_access_time: FileTime,
    _last_write_time: FileTime,
    volume_serial: u32,
    _file_size_high: u32,
    _file_size_low: u32,
    _links: u32,
    file_index_high: u32,
    file_index_low: u32,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetFileInformationByHandle(
        file: *mut std::ffi::c_void,
        information: *mut ByHandleFileInformation,
    ) -> i32;
}

/// Whether a canonical path names NTFS's deleted-files area — where the
/// volume keeps a file deleted while a handle still holds it. A concurrent
/// atomic replace (a rename over a file another handle has open) leaves the
/// old file there for an instant, and `GetFinalPathNameByHandle` answers with
/// that name instead of the workspace path. That is the file the walk just
/// validated being replaced, not an escape, so the containment check reads it
/// as an identity change rather than a path outside the root.
pub fn is_deleted_file_area(path: &Path) -> bool {
    let mut parts = path
        .components()
        .skip_while(|component| matches!(component, Component::Prefix(_) | Component::RootDir));
    let mut next_is = |name: &str| {
        parts
            .next()
            .is_some_and(|component| component.as_os_str().eq_ignore_ascii_case(name))
    };
    path.has_root() && next_is("$Extend") && next_is("$Deleted")
}

/// Whether an error from resolving a path is the transient refusal a
/// concurrent replace gives: the file the walk just validated is being
/// deleted out from under the open, so the name is momentarily unavailable.
/// Windows reports that as access denied (`STATUS_DELETE_PENDING`), delete
/// pending, a sharing violation, or a plain not-found from the window.
pub fn is_transient_replacement_error(error: &io::Error) -> bool {
    // ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_ACCESS_DENIED (how
    // STATUS_DELETE_PENDING surfaces), ERROR_SHARING_VIOLATION, and
    // ERROR_DELETE_PENDING.
    matches!(error.raw_os_error(), Some(2 | 3 | 5 | 32 | 303))
}

/// Returns the volume serial plus file index for the object this handle names.
pub(crate) fn identity(file: &File) -> io::Result<(u64, u64)> {
    let information = information(file)?;
    Ok((
        u64::from(information.volume_serial),
        (u64::from(information.file_index_high) << 32) | u64::from(information.file_index_low),
    ))
}

/// Whether this handle names a reparse point rather than an ordinary file.
pub fn is_reparse_point(file: &File) -> io::Result<bool> {
    Ok(information(file)?.attributes & 0x0000_0400 != 0)
}

/// Whether the entry at `path` is a reparse point, opening the link itself so
/// the check cannot follow it. Directories need backup semantics.
pub fn path_is_reparse_point(path: &Path) -> io::Result<bool> {
    let file = open_object(path)?;
    is_reparse_point(&file)
}

/// The identity of the directory at `path`, opened as the link itself and
/// refused when it is a reparse point or not a directory. Used to pin a
/// staged snapshot directory and re-check the name before publication.
pub fn directory_identity(path: &Path) -> io::Result<(u64, u64)> {
    let file = open_object(path)?;
    if is_reparse_point(&file)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "directory is a reparse point",
        ));
    }
    if !file.metadata()?.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "identity path is not a directory",
        ));
    }
    identity(&file)
}

fn open_object(path: &Path) -> io::Result<File> {
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
}

fn information(file: &File) -> io::Result<ByHandleFileInformation> {
    let mut information = std::mem::MaybeUninit::<ByHandleFileInformation>::uninit();
    let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { information.assume_init() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// The reparse check must open an ordinary directory by both spellings:
    /// the path the caller holds and the verbatim form `canonicalize`
    /// returns. Directory opens need backup semantics, and a refusal here
    /// would make every materialized directory look unsafe.
    #[test]
    fn an_ordinary_directory_is_not_a_reparse_point() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("saya-reparse-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        assert!(!path_is_reparse_point(&root).unwrap());
        let canonical = std::fs::canonicalize(&root).unwrap();
        assert!(!path_is_reparse_point(&canonical).unwrap());
        let _ = std::fs::remove_dir(&root);
    }
}
