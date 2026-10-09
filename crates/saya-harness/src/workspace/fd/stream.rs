//! The directory-entry stream: an owned `readdir` stream over a descriptor,
//! so bounded listings read entries from the directory the walk anchored
//! rather than from a path that could be swapped underneath.

use std::{
    ffi::{CStr, OsStr, OsString},
    io,
    os::{fd::RawFd, unix::ffi::OsStringExt},
};

use super::path_cstr;

/// An owned directory-entry stream over a descriptor, for bounded listings
/// that never re-resolve a path: entries are read from the descriptor the
/// walk anchored, not from a path that could be swapped underneath.
pub(crate) struct DirStream {
    stream: *mut libc::DIR,
}

impl DirStream {
    /// The stream's own descriptor; entry kind/size stats resolve against it.
    pub(crate) fn dir_fd(&self) -> RawFd {
        unsafe { libc::dirfd(self.stream) }
    }

    /// Opens the directory named relative to `dir_fd` (no-follow,
    /// directory-only) and owns its stream. The fresh `openat` gets a file
    /// description of its own, so the enumeration's position never collides
    /// with the caller's descriptor.
    pub(crate) fn open_at(dir_fd: RawFd, name: &OsStr) -> io::Result<Self> {
        let fd = unsafe {
            libc::openat(
                dir_fd,
                path_cstr(name)?.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let stream = unsafe { libc::fdopendir(fd) };
        if stream.is_null() {
            let error = io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(error);
        }
        Ok(Self { stream })
    }

    /// The next entry's name and raw `d_type`, or `None` at the end. The
    /// `.` and `..` self-references are skipped, as a path-based directory
    /// read reports none of them. Kind/size resolution (with a `DT_UNKNOWN`
    /// fallback) belongs to the caller holding the descriptor.
    pub(crate) fn next(&mut self) -> io::Result<Option<(OsString, u8)>> {
        loop {
            set_errno(0);
            let entry = unsafe { libc::readdir(self.stream) };
            if entry.is_null() {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(0) {
                    return Err(error);
                }
                return Ok(None);
            }
            let entry = unsafe { &*entry };
            let name = unsafe { CStr::from_ptr(entry.d_name.as_ptr()) };
            let bytes = name.to_bytes();
            if bytes == b"." || bytes == b".." {
                continue;
            }
            return Ok(Some((OsString::from_vec(bytes.to_vec()), entry.d_type)));
        }
    }
}

impl Drop for DirStream {
    fn drop(&mut self) {
        unsafe { libc::closedir(self.stream) };
    }
}

/// `readdir` returns null at end-of-stream and on error alike; clearing
/// errno first (per-platform accessor) tells the two apart.
fn set_errno(value: i32) {
    #[cfg(target_os = "macos")]
    unsafe {
        *libc::__error() = value;
    }
    #[cfg(not(target_os = "macos"))]
    unsafe {
        *libc::__errno_location() = value;
    }
}
