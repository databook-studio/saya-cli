//! Exact, anchored empty-directory cleanup for one newly owned canary namespace.
use std::{
    ffi::{CStr, CString},
    fs::{File, OpenOptions},
    io,
    mem::MaybeUninit,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::OpenOptionsExt,
    },
    path::{Component, Path, PathBuf},
};

const TARGET: &CStr = c"created-by-canary";
const DIRECTORY_FLAGS: i32 =
    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Identity(libc::dev_t, libc::ino_t);

pub(in crate::runner::sandbox) struct OwnedProbeDirectory {
    parent: File,
    directory: File,
    name: CString,
    identity: Identity,
    target: PathBuf,
    created_target: Option<Identity>,
    cleaned: Option<bool>,
}

impl OwnedProbeDirectory {
    pub(in crate::runner::sandbox) fn create(root: &Path, name: &str) -> io::Result<Self> {
        if !matches!(
            Path::new(name).components().next(),
            Some(Component::Normal(_))
        ) || Path::new(name).components().count() != 1
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid canary name",
            ));
        }
        let name_c = CString::new(name)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid canary name"))?;
        let parent = OpenOptions::new()
            .read(true)
            .custom_flags(DIRECTORY_FLAGS)
            .open(root)?;
        // SAFETY: parent is a held directory and name_c is a single terminated component.
        if unsafe { libc::mkdirat(parent.as_raw_fd(), name_c.as_ptr(), 0o700) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // A preexisting namespace is never claimed or removed. If opening the
        // newly created namespace fails, retain it rather than guess ownership.
        // SAFETY: the borrowed directory descriptor and terminated name are live.
        let descriptor =
            unsafe { libc::openat(parent.as_raw_fd(), name_c.as_ptr(), DIRECTORY_FLAGS) };
        if descriptor < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: openat returned a fresh owned descriptor above.
        let directory = unsafe { File::from_raw_fd(descriptor) };
        let identity = directory_identity(&directory, None)?;
        Ok(Self {
            parent,
            directory,
            name: name_c,
            identity,
            target: root.join(name).join("created-by-canary"),
            created_target: None,
            cleaned: None,
        })
    }

    pub(in crate::runner::sandbox) fn target(&self) -> &Path {
        &self.target
    }

    /// Called only after the actual joined canary proved its successful mkdir.
    pub(in crate::runner::sandbox) fn record_created_target(&mut self) -> bool {
        if !self.namespace_matches() {
            return false;
        }
        self.created_target = directory_identity(&self.directory, Some(TARGET)).ok();
        self.created_target.is_some()
    }

    fn namespace_matches(&self) -> bool {
        directory_identity(&self.parent, Some(&self.name)).is_ok_and(|value| value == self.identity)
    }

    pub(in crate::runner::sandbox) fn cleanup(&mut self) -> bool {
        if let Some(result) = self.cleaned {
            return result;
        }
        // Only one attempt: Drop must never remove a later replacement.
        self.cleaned = Some(false);
        if !self.namespace_matches() {
            return false;
        }
        let child = directory_identity(&self.directory, Some(TARGET));
        match (self.created_target, child) {
            (Some(expected), Ok(actual)) if expected == actual => {
                if !remove_empty_directory(&self.directory, TARGET) {
                    return false;
                }
            }
            (_, Err(error)) if error.kind() == io::ErrorKind::NotFound => {}
            _ => return false,
        }
        // The held parent/child anchors prevent traversal into replacements.
        // POSIX offers no atomic inode-conditional rmdir; a noncooperating
        // rename can still race this final identity check and name removal.
        let removed = self.namespace_matches() && remove_empty_directory(&self.parent, &self.name);
        self.cleaned = Some(removed);
        removed
    }
}

fn directory_identity(directory: &File, name: Option<&CStr>) -> io::Result<Identity> {
    let mut metadata = MaybeUninit::<libc::stat>::uninit();
    // SAFETY: descriptor is borrowed from a live File; names are terminated;
    // both operations initialize the stat structure only on success.
    let result = unsafe {
        match name {
            Some(name) => libc::fstatat(
                directory.as_raw_fd(),
                name.as_ptr(),
                metadata.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            ),
            None => libc::fstat(directory.as_raw_fd(), metadata.as_mut_ptr()),
        }
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the stat operation succeeded above.
    let metadata = unsafe { metadata.assume_init() };
    if metadata.st_mode & libc::S_IFMT != libc::S_IFDIR {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "canary entry is not a directory",
        ));
    }
    Ok(Identity(metadata.st_dev, metadata.st_ino))
}

fn remove_empty_directory(parent: &File, name: &CStr) -> bool {
    // SAFETY: held parent descriptor and terminated child name remain live.
    unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) == 0 }
}

impl Drop for OwnedProbeDirectory {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

#[cfg(test)]
#[path = "probe_entry_tests.rs"]
mod tests;
