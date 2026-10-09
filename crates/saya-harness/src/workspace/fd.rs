//! The raw descriptor mechanics the anchored walk is built on: `openat`,
//! `fstatat`, `mkdirat`, `renameat`, `unlinkat`, and a directory-entry stream.
//! Every open gains `O_NOFOLLOW` and `O_CLOEXEC` here, so no caller can
//! forget them, and no call re-resolves a path — everything is relative to a
//! descriptor for its parent.

mod stat;
mod stream;

use std::{
    ffi::{CString, OsStr},
    fs, io,
    os::{
        fd::{FromRawFd, OwnedFd, RawFd},
        unix::ffi::OsStrExt,
    },
};

pub(crate) use stat::{FinalStat, stat_at};
pub(crate) use stream::DirStream;

/// Opens the workspace root once — the descriptor every walk starts from.
pub(crate) fn open_root_fd(path: &OsStr) -> io::Result<OwnedFd> {
    let c = path_cstr(path)?;
    let fd = unsafe {
        libc::openat(
            libc::AT_FDCWD,
            c.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Opens a directory component relative to `dir_fd`: no-follow and
/// directory-only, so a symlinked component is a refusal and a non-directory
/// component is an invalid path, with no path re-resolution in between.
pub(crate) fn open_dir_at(dir_fd: RawFd, name: &OsStr) -> io::Result<OwnedFd> {
    let c = path_cstr(name)?;
    let fd = unsafe {
        libc::openat(
            dir_fd,
            c.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Opens a file component relative to `dir_fd`; every seam open gains
/// `O_NOFOLLOW` and `O_CLOEXEC` here, so no caller can forget them.
pub(crate) fn open_file_at(
    dir_fd: RawFd,
    name: &OsStr,
    flags: i32,
    mode: u32,
) -> io::Result<fs::File> {
    let c = path_cstr(name)?;
    let fd = unsafe {
        libc::openat(
            dir_fd,
            c.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            mode,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { fs::File::from_raw_fd(fd) })
}

pub(crate) fn mkdir_at(dir_fd: RawFd, name: &OsStr) -> io::Result<()> {
    let c = path_cstr(name)?;
    // The kernel applies the umask to this mode, exactly as `fs::create_dir`
    // would; creation races are re-checked by the walk's no-follow reopen.
    let rc = unsafe { libc::mkdirat(dir_fd, c.as_ptr(), 0o777) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(crate) fn rename_at(
    old_dir: RawFd,
    old: &OsStr,
    new_dir: RawFd,
    new: &OsStr,
) -> io::Result<()> {
    let old_c = path_cstr(old)?;
    let new_c = path_cstr(new)?;
    let rc = unsafe { libc::renameat(old_dir, old_c.as_ptr(), new_dir, new_c.as_ptr()) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(crate) fn unlink_at(dir_fd: RawFd, name: &OsStr) -> io::Result<()> {
    let c = path_cstr(name)?;
    let rc = unsafe { libc::unlinkat(dir_fd, c.as_ptr(), 0) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Component names are NUL-free by argument validation; a NUL here is a
/// bug, not an input, and fails closed.
fn path_cstr(name: &OsStr) -> io::Result<CString> {
    CString::new(name.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in component"))
}
