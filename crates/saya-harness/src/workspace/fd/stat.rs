//! The stat side of the descriptor mechanics: `fstatat` with
//! `AT_SYMLINK_NOFOLLOW`, and the raw `stat` fields the containment checks
//! compare.

use std::{
    ffi::OsStr,
    io, mem,
    os::fd::RawFd,
    time::{Duration, SystemTime},
};

use super::path_cstr;

/// The final component's `fstatat(AT_SYMLINK_NOFOLLOW)` result: the fields
/// the checks compare (`std::fs::Metadata` cannot be built from a raw `stat`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct FinalStat {
    dev: u64,
    ino: u64,
    mode: u32,
    size: u64,
    modified: Option<SystemTime>,
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

    /// The entry's modification time, or `None` when the stat reports one the
    /// clock cannot name — a pre-epoch second, or a nanosecond field out of
    /// range. This is the same stat that already served the kind and the size,
    /// so the time costs no second call; a caller that gets `None` falls back
    /// rather than failing.
    pub(crate) fn modified(&self) -> Option<SystemTime> {
        self.modified
    }

    fn from_raw(stat: &libc::stat) -> Self {
        Self {
            dev: stat_dev(stat.st_dev),
            ino: stat.st_ino,
            mode: stat_mode(stat.st_mode),
            size: stat.st_size.max(0) as u64,
            modified: {
                let (seconds, nanos) = stat_modified(stat);
                unix_time(seconds, nanos)
            },
        }
    }
}

/// The modification time's two raw fields, as the clock's own integers.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn stat_modified(stat: &libc::stat) -> (i64, i64) {
    (stat.st_mtime, stat.st_mtime_nsec)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn stat_modified(stat: &libc::stat) -> (i64, i64) {
    (stat.st_mtime as i64, stat.st_mtime_nsec as i64)
}

fn unix_time(seconds: i64, nanos: i64) -> Option<SystemTime> {
    let seconds = u64::try_from(seconds).ok()?;
    let nanos = u32::try_from(nanos).ok()?;
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(seconds))?
        .checked_add(Duration::from_nanos(u64::from(nanos)))
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

pub(crate) fn stat_at(dir_fd: RawFd, name: &OsStr) -> io::Result<FinalStat> {
    let c = path_cstr(name)?;
    let mut stat: libc::stat = unsafe { mem::zeroed() };
    let rc = unsafe { libc::fstatat(dir_fd, c.as_ptr(), &mut stat, libc::AT_SYMLINK_NOFOLLOW) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(FinalStat::from_raw(&stat))
}
