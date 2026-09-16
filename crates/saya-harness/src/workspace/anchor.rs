//! The anchor: one component-walk result and the operations it backs. An
//! anchor holds a descriptor for the directory the walk validated as the
//! target's parent, the final component's name relative to it, the walk's
//! own record of the path, and the final component's no-follow scan. Every
//! later step — the contained open, the temp creation, the rename, the
//! removal — is anchored to that descriptor, so it acts on the directory the
//! walk resolved regardless of what the path names now.

use std::{
    ffi::{OsStr, OsString},
    fs, io,
    os::{
        fd::{AsRawFd, OwnedFd},
        unix::fs::{MetadataExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    process,
    sync::Arc,
};

use super::contain::nanos;
use super::fd::{FinalStat, open_file_at, rename_at, stat_at, unlink_at};
use crate::{HarnessError, io_error};

/// One component-walk result: a descriptor for the directory the walk
/// validated as the target's parent, the final component's name relative to
/// it, the path as walked (for diagnostics), and the final component's
/// no-follow scan — `None` when it does not exist yet. Every later step
/// (open, temp creation, rename) is anchored to `dir`, so it acts on the
/// directory the walk resolved regardless of what the path names now.
pub(crate) struct Anchor {
    dir: Arc<OwnedFd>,
    name: OsString,
    path: PathBuf,
    stat: Option<FinalStat>,
}

impl Anchor {
    /// Assembles the walk's result; only the walk constructs anchors.
    pub(crate) fn new(
        dir: Arc<OwnedFd>,
        name: OsString,
        path: PathBuf,
        stat: Option<FinalStat>,
    ) -> Self {
        Self {
            dir,
            name,
            path,
            stat,
        }
    }

    /// The path as walked — for error display and callers that need a
    /// `PathBuf`; never re-resolved for containment purposes.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// The anchored parent descriptor's raw fd.
    pub(crate) fn dir_fd(&self) -> std::os::fd::RawFd {
        self.dir.as_raw_fd()
    }

    /// The final component's name, relative to the anchored parent.
    pub(crate) fn name(&self) -> &OsStr {
        &self.name
    }

    /// The final component's no-follow scan, `None` when it is absent.
    pub(crate) fn stat(&self) -> Option<&FinalStat> {
        self.stat.as_ref()
    }

    /// A fresh `fstatat(AT_SYMLINK_NOFOLLOW)` against the anchored parent,
    /// for post-write verification.
    pub(crate) fn stat_final(&self) -> io::Result<FinalStat> {
        stat_at(self.dir.as_raw_fd(), &self.name)
    }

    /// Opens the final component relative to the anchored parent. The open is
    /// always no-follow and close-on-exec; a link swapped into the final
    /// component fails the open itself.
    pub(crate) fn open_file(&self, flags: i32, mode: u32) -> io::Result<fs::File> {
        open_file_at(self.dir.as_raw_fd(), &self.name, flags, mode)
    }

    /// The contained read open: no-follow, then a post-open identity check
    /// against the scan `expected` — the opened file's (dev, inode) must be
    /// the one that was scanned.
    pub(crate) fn open_verified(
        &self,
        expected: (u64, u64),
        rel: &str,
    ) -> Result<fs::File, HarnessError> {
        let file = self.open_file(libc::O_RDONLY, 0).map_err(|error| {
            if error.raw_os_error() == Some(libc::ELOOP) {
                return HarnessError::SymlinkRefused {
                    path: rel.to_string(),
                };
            }
            io_error("open workspace file", &self.path, error)
        })?;
        let opened = file
            .metadata()
            .map_err(|error| io_error("stat opened workspace file", &self.path, error))?;
        if (opened.dev(), opened.ino()) != expected {
            return Err(HarnessError::IdentityChanged {
                path: rel.to_string(),
            });
        }
        Ok(file)
    }

    /// Opens a fresh temp file at 0600 inside the anchored parent with
    /// `O_CREAT|O_EXCL`, so a pre-planted name (even a link) can never be
    /// adopted. Returns the temp's name relative to the anchored parent.
    pub(crate) fn create_temp(&self, rel: &str) -> Result<(OsString, fs::File), HarnessError> {
        for attempt in 0..8 {
            let name = format!(".saya-tmp-{}-{}-{attempt}", process::id(), nanos());
            let candidate = self.path.join(&name);
            match open_file_at(
                self.dir.as_raw_fd(),
                OsStr::new(&name),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                0o600,
            ) {
                Ok(file) => return Ok((OsString::from(name), file)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(io_error("create workspace temp", &candidate, error)),
            }
        }
        Err(HarnessError::Io {
            context: format!("name a unique workspace temp file for {rel}"),
            source: io::Error::new(io::ErrorKind::AlreadyExists, "temp name collisions"),
        })
    }

    /// Renames `from` (a name in the anchored parent) over the final
    /// component — the atomic write's last step, anchored to the descriptor
    /// rather than re-resolved by path.
    pub(crate) fn rename_over(&self, from: &OsStr) -> Result<(), HarnessError> {
        rename_at(self.dir.as_raw_fd(), from, self.dir.as_raw_fd(), &self.name)
            .map_err(|error| io_error("replace workspace file", &self.path, error))
    }

    /// Commits `bytes` as the final component's new content: temp file at
    /// 0600 inside the anchored parent, fsync, anchored rename over the
    /// target, then post-write re-verification that the destination still
    /// holds what was placed. Shared by the whole-file write and the range
    /// patch so both commit through one path; anything before the rename
    /// leaves the old content untouched, never a partial file.
    pub(crate) fn commit_bytes(&self, rel: &str, bytes: &[u8]) -> Result<(), HarnessError> {
        use std::io::Write as _;
        let (temp_name, mut temp) = self.create_temp(rel)?;
        let temp_path = self.path.join(&temp_name);
        temp.write_all(bytes)
            .map_err(|error| io_error("write workspace temp", &temp_path, error))?;
        temp.sync_all()
            .map_err(|error| io_error("sync workspace temp", &temp_path, error))?;
        let written = {
            let meta = temp
                .metadata()
                .map_err(|error| io_error("stat workspace temp", &temp_path, error))?;
            (meta.dev(), meta.ino())
        };
        temp.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| io_error("set mode on", &temp_path, error))?;
        drop(temp);
        self.rename_over(&temp_name)?;
        let final_stat = self
            .stat_final()
            .map_err(|error| io_error("verify written workspace file", &self.path, error))?;
        if final_stat.identity() != written || !final_stat.is_file() || final_stat.has_exec_bits() {
            return Err(HarnessError::IdentityChanged {
                path: rel.to_string(),
            });
        }
        Ok(())
    }

    /// Renames another anchor's final component over this anchor's — the
    /// download promote: part and destination may sit in different anchored
    /// directories of the same tree.
    pub(crate) fn rename_from(&self, source: &Anchor) -> Result<(), HarnessError> {
        rename_at(
            source.dir.as_raw_fd(),
            &source.name,
            self.dir.as_raw_fd(),
            &self.name,
        )
        .map_err(|error| io_error("replace workspace file", &self.path, error))
    }

    /// Removes the final component relative to the anchored parent.
    pub(crate) fn unlink_final(&self) -> io::Result<()> {
        unlink_at(self.dir.as_raw_fd(), &self.name)
    }
}
