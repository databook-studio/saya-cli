//! The anchored containment walk (unix): every path component is resolved
//! relative to a file descriptor for its parent — [`open_dir_at`] for
//! directories, [`stat_at`] for the final component, [`mkdir_at`] for
//! components the caller may create — instead of re-resolving a cumulative
//! path at every step. A swap of an intermediate directory between any two
//! steps therefore cannot divert resolution: the next step reads from the
//! descriptor the walk actually holds, and the open, the temp creation, and
//! the rename all act on the directory the walk validated. The root is
//! pinned the same way, by a descriptor opened once at [`Workspace::open`].

use std::{
    ffi::{OsStr, OsString},
    io,
    os::fd::{AsRawFd, OwnedFd},
    path::Path,
    sync::Arc,
};

use super::anchor::Anchor;
use super::contain::{Workspace, argument_components};
use super::fd::{mkdir_at, open_dir_at, stat_at};
use crate::{HarnessError, io_error};

impl Workspace {
    /// The anchored component walk: argument validation, then each component
    /// resolved relative to a descriptor for its parent — refusing symlinks
    /// at every component, creating missing directories only when
    /// `allow_create`, and never letting `..` rise above the root. The walk's
    /// own record of the path is kept for error display and for callers that
    /// need a `PathBuf`; containment never re-resolves it.
    pub(crate) fn anchor(&self, rel: &str, allow_create: bool) -> Result<Anchor, HarnessError> {
        let comps = argument_components(rel)?;
        let mut dir: Arc<OwnedFd> = Arc::clone(self.root_dir_fd());
        let mut path = self.root().to_path_buf();
        let mut depth = 0usize;
        for (index, comp) in comps.iter().enumerate() {
            let last = index + 1 == comps.len();
            if comp == ".." {
                if depth == 0 {
                    return Err(HarnessError::PathOutsideRoot {
                        path: rel.to_string(),
                    });
                }
                depth -= 1;
                let opened = open_dir_at(dir.as_raw_fd(), OsStr::new(".."))
                    .map_err(|error| io_error("scan workspace path", &path, error))?;
                dir = Arc::new(opened);
                path.pop();
                continue;
            }
            if last {
                path.push(comp);
                break;
            }
            path.push(comp);
            match open_dir_at(dir.as_raw_fd(), OsStr::new(comp)) {
                Ok(opened) => {
                    dir = Arc::new(opened);
                    depth += 1;
                }
                Err(error) => {
                    // Classify the failure the way the path walk classified
                    // every component: against a no-follow stat of the
                    // component itself, not against the open's errno — a
                    // kernel may report a refused link as ELOOP or ENOTDIR,
                    // but the stat's verdict is the same on both.
                    let scan = stat_at(dir.as_raw_fd(), OsStr::new(comp));
                    if let Ok(stat) = scan {
                        if stat.is_symlink() {
                            return Err(HarnessError::SymlinkRefused {
                                path: rel.to_string(),
                            });
                        }
                        if !stat.is_dir() {
                            return Err(HarnessError::InvalidPath {
                                path: rel.to_string(),
                            });
                        }
                        // A real directory that failed to open: the old
                        // walk would have proceeded and failed at the next
                        // component with the same permission error.
                        return Err(io_error("scan workspace path", &path, error));
                    }
                    if error.kind() != io::ErrorKind::NotFound {
                        return Err(io_error("scan workspace path", &path, error));
                    }
                    if !allow_create {
                        return Err(io_error("scan workspace path", &path, error));
                    }
                    // Lost a creation race to a winner that must be a real
                    // directory: the no-follow, directory-only reopen maps a
                    // link to a refusal and a file to an invalid path,
                    // exactly as the path walk's re-check did.
                    mkdir_at(dir.as_raw_fd(), OsStr::new(comp))
                        .map_err(|error| io_error("scan workspace path", &path, error))?;
                    let opened =
                        open_dir_at(dir.as_raw_fd(), OsStr::new(comp)).map_err(|error| {
                            classify(&path, rel, dir.as_raw_fd(), OsStr::new(comp), error)
                        })?;
                    dir = Arc::new(opened);
                    depth += 1;
                }
            }
        }
        // A `..`-terminated walk ends on a directory, not a name; the final
        // component of that directory is itself.
        let name = if comps.last().map(String::as_str) == Some("..") {
            OsString::from(".")
        } else {
            comps
                .last()
                .expect("validated arguments are non-empty")
                .into()
        };
        let stat = match stat_at(dir.as_raw_fd(), &name) {
            Ok(stat) if stat.is_symlink() => {
                return Err(HarnessError::SymlinkRefused {
                    path: rel.to_string(),
                });
            }
            Ok(stat) => Some(stat),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(io_error("scan workspace path", &path, error)),
        };
        Ok(Anchor::new(dir, name, path, stat))
    }
}

/// Maps a component open's failure to the walk's refusal classes the way the
/// path walk classified every component: against a no-follow stat of the
/// component itself, not against the open's errno — a kernel may report a
/// refused link as ELOOP or ENOTDIR, but the stat's verdict is the same on
/// both. A vanished component keeps the original error.
fn classify(
    path: &Path,
    rel: &str,
    dir_fd: std::os::fd::RawFd,
    comp: &OsStr,
    error: io::Error,
) -> HarnessError {
    if let Ok(stat) = stat_at(dir_fd, comp) {
        if stat.is_symlink() {
            return HarnessError::SymlinkRefused {
                path: rel.to_string(),
            };
        }
        if !stat.is_dir() {
            return HarnessError::InvalidPath {
                path: rel.to_string(),
            };
        }
    }
    io_error("scan workspace path", path, error)
}
