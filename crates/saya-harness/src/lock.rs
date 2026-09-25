//! Single-writer claim on a run directory: a pid lockfile. Two saya
//! instances never share a run dir (the design's single-writer rule), so the
//! engine takes a [`RunLock`] on `runs/<id>/lock` before anything else.
//!
//! A lock file whose pid names a live process refuses acquisition with a
//! diagnostic naming the holder. A stale file — dead pid, empty, or
//! unparsable — is reclaimed. On platforms without a process-liveness
//! primitive the lock fails closed: any pre-existing file is treated as a
//! live holder, so a stale lock there needs manual removal rather than a
//! silent takeover.
//!
//! Claiming writes the pid to a pid-suffixed temp file and renames it over
//! the lock path, then verifies the path now names *this* process. That
//! re-read is load-bearing: two processes that both saw a stale file race
//! their renames, and only the one whose pid the file ends up naming holds
//! the lock; the loser refuses itself.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process,
};

use crate::{HarnessError, io_error};

/// A held run lock. Dropping it is best-effort release; [`RunLock::release`]
/// reports a failed release instead.
#[derive(Debug)]
pub struct RunLock {
    path: PathBuf,
    pid: u32,
}

impl RunLock {
    /// Acquires the lock file at `path`. Refuses with [`HarnessError::LockHeld`]
    /// while a live process holds it; reclaims a stale file.
    pub fn acquire(path: impl AsRef<Path>) -> Result<Self, HarnessError> {
        let path = path.as_ref().to_path_buf();
        let pid = process::id();
        if let Some(holder) = holder_pid(&path)? {
            if process_alive(holder) {
                return Err(HarnessError::LockHeld { pid: holder });
            }
            // The holder is gone. Reclaim; the rename below decides the race
            // if another process is reclaiming the same stale file.
            let _ = fs::remove_file(&path);
        }
        claim(&path, pid)?;
        match holder_pid(&path)? {
            Some(observed) if observed == pid => Ok(Self { path, pid }),
            Some(observed) if process_alive(observed) => {
                Err(HarnessError::LockHeld { pid: observed })
            }
            _ => Err(HarnessError::LockContended),
        }
    }

    /// The pid the lock file names: this process's.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Removes the lock file if it still names this process, leaving a
    /// successor's file alone.
    pub fn release(mut self) -> Result<(), HarnessError> {
        self.remove_if_owned()
    }

    fn remove_if_owned(&mut self) -> Result<(), HarnessError> {
        if holder_pid(&self.path)? == Some(self.pid) {
            fs::remove_file(&self.path)
                .map_err(|error| io_error("release run lock", &self.path, error))?;
        }
        Ok(())
    }
}

impl Drop for RunLock {
    fn drop(&mut self) {
        let _ = self.remove_if_owned();
    }
}

/// The pid a lock file names, or `None` when it does not exist or carries
/// anything unparsable.
fn holder_pid(path: &Path) -> Result<Option<u32>, HarnessError> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error("read run lock", path, error)),
    };
    Ok(raw.trim().parse().ok())
}

/// Writes our pid to the lock path: pid-suffixed temp file, fsync, rename —
/// the store's atomic-write pattern — so the path never holds a half-written
/// pid.
fn claim(path: &Path, pid: u32) -> Result<(), HarnessError> {
    let mut name = path
        .file_name()
        .map(std::ffi::OsString::from)
        .unwrap_or_default();
    name.push(format!(".{pid}.tmp"));
    let temp = path.with_file_name(name);
    let mut file =
        fs::File::create(&temp).map_err(|error| io_error("create run lock", &temp, error))?;
    file.write_all(format!("{pid}\n").as_bytes())
        .map_err(|error| io_error("write run lock", &temp, error))?;
    file.sync_all()
        .map_err(|error| io_error("sync run lock", &temp, error))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o600))
            .map_err(|error| io_error("set mode on run lock", &temp, error))?;
    }
    fs::rename(&temp, path).map_err(|error| io_error("claim run lock", path, error))
}

/// Whether a pid names a process that exists right now. A zero signal asks
/// the kernel only about existence; EPERM means the process exists and is
/// owned by another user.
#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    // SAFETY: `kill` takes no pointers and has no invariants to uphold.
    let sent = unsafe { libc::kill(pid as libc::pid_t, 0) };
    sent == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// No std primitive answers this off unix. Every existing file is treated as
/// a live holder — fail closed: a stale lock there needs manual removal
/// rather than a silent reclaim by a second writer.
#[cfg(not(unix))]
fn process_alive(_pid: u32) -> bool {
    true
}
