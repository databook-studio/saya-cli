//! The raw descriptor mechanics the anchored walk is built on: `openat`,
//! `fstatat`, `mkdirat`, `renameat`, `unlinkat`, and a directory-entry stream.
//! Every open gains `O_NOFOLLOW` and `O_CLOEXEC` here, so no caller can
//! forget them, and no call re-resolves a path — everything is relative to a
//! descriptor for its parent.

use std::{
    ffi::{CStr, CString, OsStr, OsString},
    fs, io, mem,
    os::{
        fd::{FromRawFd, OwnedFd, RawFd},
        unix::ffi::{OsStrExt, OsStringExt},
    },
};

/// The final component's `fstatat(AT_SYMLINK_NOFOLLOW)` result: the fields
/// the checks compare (`std::fs::Metadata` cannot be built from a raw `stat`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct FinalStat {
    dev: u64,
    ino: u64,
    mode: u32,
    size: u64,
}

impl FinalStat {
    pub(crate) fn identity(&self) -> (u64, u64) {
        (self.dev, self.ino)
    }

    pub(crate) fn is_symlink(&self) -> bool {
        self.mode & stat_mode(libc::S_IFMT) == stat_mode(libc::S_IFLNK)
    }

    pub(crate) fn is_file(&self) -> bool {
        self.mode & stat_mode(libc::S_IFMT) == stat_mode(libc::S_IFREG)
    }

    pub(crate) fn is_dir(&self) -> bool {
        self.mode & stat_mode(libc::S_IFMT) == stat_mode(libc::S_IFDIR)
    }

    pub(crate) fn len(&self) -> u64 {
        self.size
    }

    pub(crate) fn has_exec_bits(&self) -> bool {
        self.mode & 0o111 != 0
    }

    fn from_raw(stat: &libc::stat) -> Self {
        Self {
            dev: stat_dev(stat.st_dev),
            ino: stat.st_ino,
            mode: stat_mode(stat.st_mode),
            size: stat.st_size.max(0) as u64,
        }
    }
}

#[cfg(target_os = "linux")]
fn stat_mode(mode: libc::mode_t) -> u32 {
    mode
}

#[cfg(target_os = "macos")]
fn stat_mode(mode: libc::mode_t) -> u32 {
    u32::from(mode)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn stat_mode(mode: libc::mode_t) -> u32 {
    mode as u32
}

#[cfg(target_os = "linux")]
fn stat_dev(dev: libc::dev_t) -> u64 {
    dev
}

#[cfg(target_os = "macos")]
fn stat_dev(dev: libc::dev_t) -> u64 {
    dev as u64
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn stat_dev(dev: libc::dev_t) -> u64 {
    dev as u64
}

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

pub(crate) fn stat_at(dir_fd: RawFd, name: &OsStr) -> io::Result<FinalStat> {
    let c = path_cstr(name)?;
    let mut stat: libc::stat = unsafe { mem::zeroed() };
    let rc = unsafe { libc::fstatat(dir_fd, c.as_ptr(), &mut stat, libc::AT_SYMLINK_NOFOLLOW) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(FinalStat::from_raw(&stat))
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

/// Component names are NUL-free by argument validation; a NUL here is a
/// bug, not an input, and fails closed.
fn path_cstr(name: &OsStr) -> io::Result<CString> {
    CString::new(name.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in component"))
}
