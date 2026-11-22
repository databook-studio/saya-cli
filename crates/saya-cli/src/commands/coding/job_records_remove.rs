//! The unix half of a job-record removal: the tombstoned tree is taken through
//! directory file descriptors, never through a path.
//!
//! A walk that classified a path and then entered it would have a window in
//! which a directory swapped for a symlink redirects the walk at whatever the
//! link names. So the tombstone is opened once with `O_DIRECTORY | O_NOFOLLOW`,
//! and every child is entered with `openat` under the same flags, classified
//! with `fstatat(.., AT_SYMLINK_NOFOLLOW)`, and removed by name with `unlinkat`.
//! A link — at a child's name, or swapped in between the classification and the
//! open — is unlinked as a name and its target is never reached. The bounds are
//! the walk's own: [`MAX_DEPTH`] and [`MAX_ENTRIES`] per directory, and a bound
//! that trips leaves the tombstone for the next delete rather than a half job.
//!
//! Every unsafe block here is a libc call over a descriptor this module owns,
//! for the length of that call.

use super::{
    CodingError,
    job_records::{MAX_DEPTH, MAX_ENTRIES},
};
use std::{
    ffi::{CStr, CString},
    fs, io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
    os::unix::fs::OpenOptionsExt,
    path::Path,
};

/// What a name holds, as the walk classifies it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Enters {
    /// A real directory: entered with `openat` under `O_NOFOLLOW`.
    Directory,
    /// Anything else — a file, a link, a device: removed by its name alone.
    Name,
}

/// Removes the entry `name` under `parent`, one of a job's tombstones. An entry
/// that is already gone is already removed.
pub(super) fn remove(parent: &Path, name: &str) -> Result<(), CodingError> {
    let Some(directory) = open_directory(parent)? else {
        return Ok(());
    };
    let fd = directory.as_raw_fd();
    // The tombstone's own name is opened before anything is classified: the
    // `O_NOFOLLOW` above the walk is what keeps a link planted at that name a
    // name and not a way into the folder it points at.
    match open_child(fd, name)? {
        Some(child) => {
            remove_contents(child.as_raw_fd(), MAX_DEPTH)?;
            unlink(fd, name, true)
        }
        None => match classify(fd, name)? {
            None => Ok(()),
            // Swapped for a link, or a plain file, since it was renamed: the
            // name goes, whatever it is now, and its target is untouched.
            Some(_) => unlink(fd, name, false),
        },
    }
}

/// The directory at `path`, or `None` when it is already gone. `O_NOFOLLOW`
/// refuses a link planted at the path itself.
fn open_directory(path: &Path) -> Result<Option<OwnedFd>, CodingError> {
    let mut options = fs::OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
    match options.open(path) {
        Ok(file) => Ok(Some(file.into())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(CodingError::owner(error)),
    }
}

/// The directory `name` holds under `dirfd`, or `None` when it is not one now:
/// `O_NOFOLLOW` fails a link, `O_DIRECTORY` a file, and both are the caller's
/// cue to remove the name instead of entering it.
fn open_child(dirfd: RawFd, name: &str) -> Result<Option<OwnedFd>, CodingError> {
    let name = cstring(name)?;
    // SAFETY: `dirfd` is an open directory descriptor for the call and `name` is
    // a NUL-terminated string that outlives it.
    let opened = unsafe {
        libc::openat(
            dirfd,
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if opened >= 0 {
        // SAFETY: `opened` is a descriptor opened by the call above, which this
        // function hands to the owner that closes it.
        return Ok(Some(unsafe { OwnedFd::from_raw_fd(opened) }));
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ELOOP | libc::ENOTDIR | libc::ENOENT) => Ok(None),
        _ => Err(CodingError::owner(error)),
    }
}

/// What `name` holds under `dirfd`, read without following a link.
fn classify(dirfd: RawFd, name: &str) -> Result<Option<Enters>, CodingError> {
    let name = cstring(name)?;
    // SAFETY: `fstatat` writes the buffer it is given and nothing reads it
    // unless it reports success; `dirfd` and `name` are valid for the call.
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    let queried = unsafe {
        libc::fstatat(
            dirfd,
            name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if queried != 0 {
        let error = io::Error::last_os_error();
        return match error.kind() {
            io::ErrorKind::NotFound => Ok(None),
            _ => Err(CodingError::owner(error)),
        };
    }
    // SAFETY: `fstatat` reported success, so it initialized the buffer.
    let stat = unsafe { stat.assume_init() };
    Ok(Some(if (stat.st_mode & libc::S_IFMT) == libc::S_IFDIR {
        Enters::Directory
    } else {
        Enters::Name
    }))
}

/// Removes the name `name` under `dirfd`: the entry itself, never what a link
/// points at. An absent name is already removed.
fn unlink(dirfd: RawFd, name: &str, directory: bool) -> Result<(), CodingError> {
    let name = cstring(name)?;
    let flags = if directory { libc::AT_REMOVEDIR } else { 0 };
    // SAFETY: `dirfd` is an open directory descriptor for the call and `name` is
    // a NUL-terminated string that outlives it.
    let removed = unsafe { libc::unlinkat(dirfd, name.as_ptr(), flags) };
    if removed == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::NotFound {
        return Ok(());
    }
    Err(CodingError::owner(error))
}

/// Empties the directory `dirfd` names, one level of the walk. What a pass
/// removes stays removed, so a directory past the bounds converges on the next
/// pass.
fn remove_contents(dirfd: RawFd, depth: usize) -> Result<(), CodingError> {
    if depth == 0 {
        return Err(CodingError::StateDirectory);
    }
    let (names, cut) = read_names(dirfd)?;
    for name in &names {
        match classify(dirfd, name)? {
            None => {}
            Some(Enters::Name) => unlink(dirfd, name, false)?,
            Some(Enters::Directory) => match open_child(dirfd, name)? {
                Some(child) => {
                    remove_contents(child.as_raw_fd(), depth - 1)?;
                    unlink(dirfd, name, true)?;
                }
                None => unlink(dirfd, name, false)?,
            },
        }
    }
    // More names than one pass may take: the ones read are removed, and the
    // next pass — or the next delete — takes the rest.
    if cut {
        return Err(CodingError::StateDirectory);
    }
    Ok(())
}

/// The names directly under `dirfd`, at most [`MAX_ENTRIES`], and whether the
/// directory held more than that.
fn read_names(dirfd: RawFd) -> Result<(Vec<String>, bool), CodingError> {
    // SAFETY: `dirfd` is an open directory descriptor for the call; `dup` hands
    // back a second descriptor this call owns.
    let duplicate = unsafe { libc::dup(dirfd) };
    if duplicate < 0 {
        return Err(CodingError::owner(io::Error::last_os_error()));
    }
    // SAFETY: `duplicate` is an open directory descriptor and `fdopendir` takes
    // ownership of it; the stream is closed exactly once, below.
    let stream = unsafe { libc::fdopendir(duplicate) };
    if stream.is_null() {
        let error = io::Error::last_os_error();
        // SAFETY: `fdopendir` failed, so `duplicate` is still this call's.
        unsafe { libc::close(duplicate) };
        return Err(CodingError::owner(error));
    }
    let mut names = Vec::new();
    let mut cut = false;
    loop {
        // SAFETY: `stream` is a live directory stream owned by this call.
        let entry = unsafe { libc::readdir(stream) };
        if entry.is_null() {
            break;
        }
        // SAFETY: `readdir` returned a live entry whose name is NUL-terminated
        // inside the entry's own storage; the name is copied before the next
        // call may reuse it.
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
        let Ok(name) = name.to_str() else {
            // SAFETY: `stream` is live and is not used again after this call.
            unsafe { libc::closedir(stream) };
            return Err(CodingError::StateDirectory);
        };
        if name == "." || name == ".." {
            continue;
        }
        if names.len() == MAX_ENTRIES {
            cut = true;
            break;
        }
        names.push(name.to_owned());
    }
    // SAFETY: `stream` came from `fdopendir` above and is not used again.
    unsafe { libc::closedir(stream) };
    Ok((names, cut))
}

/// `name` as a C string. A name that is not one refuses the removal.
fn cstring(name: &str) -> Result<CString, CodingError> {
    CString::new(name).map_err(|_| CodingError::StateDirectory)
}
