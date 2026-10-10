//! The Unix seam of the output store: descriptor-relative, no-follow
//! creation of the store's directories and files, the nonce-directory
//! guard, and the typed store failure. Nothing here ever chmods an
//! existing directory: pre-existing paths must already be private and are
//! refused otherwise.
//!
//! The whole module is compiled only on Unix: platforms without the
//! `openat` seam never touch it, and the store refuses preparation there
//! with a typed error and creates nothing.

use std::{
    ffi::{CString, OsStr},
    fs, io,
    os::{
        fd::{AsRawFd, OwnedFd, RawFd},
        unix::ffi::OsStrExt,
    },
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::workspace::fd::{open_dir_at, open_file_at, open_root_fd};

use super::output::{CommandOutputError, OutputStream};

/// The typed store failure: kind and raw os code only, never message text.
pub(super) fn store_error(source: io::Error) -> CommandOutputError {
    CommandOutputError::Store {
        kind: source.kind(),
        os_code: source.raw_os_error(),
    }
}

static NEXT_STORE_NONCE: AtomicU64 = AtomicU64::new(0);

/// The nonce-named leaf directory for one store attempt: nanos plus a
/// process-local counter.
pub(super) fn next_nonce_leaf() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{nanos}-{}",
        NEXT_STORE_NONCE.fetch_add(1, Ordering::Relaxed)
    )
}

/// Removes a just-created nonce directory when preparation fails after the
/// directory exists: the stream files and the directory itself are unlinked
/// through the retained `command-output` descriptor — never by path.
pub(super) struct NonceGuard {
    base_fd: RawFd,
    nonce: String,
    defused: bool,
}

impl NonceGuard {
    /// Arms the guard on the just-created nonce directory name.
    pub(super) fn armed(base_fd: RawFd, nonce: &str) -> Self {
        Self {
            base_fd,
            nonce: nonce.to_owned(),
            defused: false,
        }
    }

    pub(super) fn defuse(mut self) {
        self.defused = true;
    }
}

impl Drop for NonceGuard {
    fn drop(&mut self) {
        // Best-effort: the failed preparation returns its own error either
        // way, and the guard removes only this attempt's nonce.
        if !self.defused {
            let _ = unlink_nonce(self.base_fd, &self.nonce);
        }
    }
}

/// Opens `store_root` no-follow. An existing directory must already be
/// owned by the effective user with group and other access cleared and
/// is refused otherwise; a missing leaf is created once at 0700.
pub(super) fn open_store_root(store_root: &Path) -> io::Result<OwnedFd> {
    let leaf = store_root
        .file_name()
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    let parent = store_root
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    let parent_fd = open_root_fd(parent.as_os_str())?;
    match open_dir_at(parent_fd.as_raw_fd(), leaf) {
        Ok(fd) => {
            verify_private(fd.as_raw_fd())?;
            Ok(fd)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            mkdir_private(parent_fd.as_raw_fd(), leaf)?;
            open_private_dir(parent_fd.as_raw_fd(), leaf)
        }
        Err(error) => Err(error),
    }
}

/// Opens `name` under `dir_fd` no-follow and verifies it.
pub(super) fn open_private_dir(dir_fd: RawFd, name: &OsStr) -> io::Result<OwnedFd> {
    let fd = open_dir_at(dir_fd, name)?;
    verify_private(fd.as_raw_fd())?;
    Ok(fd)
}

/// Opens `name` under `dir_fd` no-follow, or creates it at 0700.
pub(super) fn open_or_create_private_dir(dir_fd: RawFd, name: &OsStr) -> io::Result<OwnedFd> {
    match open_dir_at(dir_fd, name) {
        Ok(fd) => {
            verify_private(fd.as_raw_fd())?;
            Ok(fd)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            mkdir_private(dir_fd, name)?;
            open_private_dir(dir_fd, name)
        }
        Err(error) => Err(error),
    }
}

/// `mkdirat` at an explicit 0700 mode; the no-follow reopen re-verifies.
pub(super) fn mkdir_private(dir_fd: RawFd, name: &OsStr) -> io::Result<()> {
    let name = cstring(name)?;
    if unsafe { libc::mkdirat(dir_fd, name.as_ptr(), 0o700) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// A pre-existing path must be owned by the effective user, private;
/// anything else is refused, never repaired.
fn verify_private(fd: RawFd) -> io::Result<()> {
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut stat) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `geteuid` takes no arguments and has no preconditions.
    if stat.st_uid != unsafe { libc::geteuid() } || stat.st_mode & 0o077 != 0 {
        return Err(io::Error::from(io::ErrorKind::PermissionDenied));
    }
    Ok(())
}

/// Exclusively creates the stream's 0600 file relative to the retained
/// nonce-directory descriptor; a pre-placed name is refused, never
/// adopted.
pub(super) fn open_store_file(dir_fd: RawFd, stream: OutputStream) -> io::Result<fs::File> {
    open_file_at(
        dir_fd,
        OsStr::new(stream.file_name()),
        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        0o600,
    )
}

fn cstring(name: &OsStr) -> io::Result<CString> {
    CString::new(name.as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
}

/// Opens one stored stream file read-only for the cursor reader: the
/// nonce directory by root fd, the stream file no-follow beneath it — a
/// pre-placed symlink fails as a typed error, never followed. The caller
/// verifies the opened descriptor is a regular file before use.
pub(super) fn open_read_file(dir: &Path, stream: OutputStream) -> io::Result<fs::File> {
    let dir_fd = open_root_fd(dir.as_os_str())?;
    open_file_at(
        dir_fd.as_raw_fd(),
        OsStr::new(stream.file_name()),
        libc::O_RDONLY | libc::O_NONBLOCK,
        0,
    )
}

/// Removes the nonce directory a failed preparation created: the stream
/// files are unlinked through a no-follow open of the nonce directory,
/// then the directory with `AT_REMOVEDIR` — all relative to the retained
/// `command-output` descriptor, never by path. Best-effort.
fn unlink_nonce(base_fd: RawFd, nonce: &str) -> io::Result<()> {
    if let Ok(dir_fd) = open_private_dir(base_fd, OsStr::new(nonce)) {
        for stream in [OutputStream::Stdout, OutputStream::Stderr] {
            let name = cstring(OsStr::new(stream.file_name()))?;
            unsafe { libc::unlinkat(dir_fd.as_raw_fd(), name.as_ptr(), 0) };
        }
    }
    let nonce_name = cstring(OsStr::new(nonce))?;
    if unsafe { libc::unlinkat(base_fd, nonce_name.as_ptr(), libc::AT_REMOVEDIR) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
