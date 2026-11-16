//! One contained directory stream shared by strict and partial search traversal.
use super::{EntryKind, ListEntry, Workspace};
use crate::{HarnessError, io_error};
#[cfg(unix)]
use std::{ffi::OsStr, os::fd::AsRawFd};

pub(super) struct Directory {
    #[cfg(unix)]
    stream: super::fd::DirStream,
    #[cfg(not(unix))]
    stream: std::fs::ReadDir,
    #[cfg(not(unix))]
    workspace: Workspace,
}
impl Directory {
    pub fn open(workspace: &Workspace, path: &str) -> Result<Self, HarnessError> {
        #[cfg(unix)]
        {
            use super::fd::{DirStream, stat_at};
            let stream = if path.is_empty() {
                DirStream::open_at(workspace.root_dir_fd().as_raw_fd(), OsStr::new("."))
            } else {
                let anchor = workspace.anchor(path, false)?;
                let stat = anchor
                    .stat()
                    .ok_or_else(|| HarnessError::NotFound { path: path.into() })?;
                if !stat.is_dir() {
                    return Err(HarnessError::NotRegularFile { path: path.into() });
                }
                let stream = DirStream::open_at(anchor.dir_fd(), anchor.name())
                    .map_err(|error| io_error("open search directory", workspace.root(), error))?;
                let opened = stat_at(stream.dir_fd(), OsStr::new(".")).map_err(|error| {
                    io_error("verify search directory", workspace.root(), error)
                })?;
                if opened.identity() != stat.identity() {
                    return Err(HarnessError::IdentityChanged { path: path.into() });
                }
                Ok(stream)
            }
            .map_err(|error| io_error("open search directory", workspace.root(), error))?;
            Ok(Self { stream })
        }
        #[cfg(not(unix))]
        {
            let target = if path.is_empty() {
                workspace.root().to_path_buf()
            } else {
                workspace.target(path, false)?
            };
            Ok(Self {
                stream: std::fs::read_dir(target)
                    .map_err(|error| io_error("open search directory", workspace.root(), error))?,
                workspace: workspace.clone(),
            })
        }
    }
    pub fn child(
        &self,
        workspace: &Workspace,
        path: &str,
        name: &str,
    ) -> Result<Self, HarnessError> {
        #[cfg(unix)]
        {
            use super::fd::{DirStream, stat_at};
            let before = stat_at(self.stream.dir_fd(), OsStr::new(name))
                .map_err(|error| io_error("stat search child", workspace.root(), error))?;
            if !before.is_dir() {
                return Err(HarnessError::NotRegularFile {
                    path: String::new(),
                });
            }
            let stream = DirStream::open_at(self.stream.dir_fd(), OsStr::new(name))
                .map_err(|error| io_error("open search child", workspace.root(), error))?;
            let opened = stat_at(stream.dir_fd(), OsStr::new("."))
                .map_err(|error| io_error("verify search child", workspace.root(), error))?;
            if before.identity() != opened.identity() {
                return Err(HarnessError::IdentityChanged {
                    path: String::new(),
                });
            }
            let _ = path;
            Ok(Self { stream })
        }
        #[cfg(not(unix))]
        {
            let _ = name;
            Self::open(workspace, path)
        }
    }
    pub fn kind(
        &self,
        workspace: &Workspace,
        path: &str,
        name: &str,
    ) -> Result<EntryKind, HarnessError> {
        #[cfg(unix)]
        {
            let stat = super::fd::stat_at(self.stream.dir_fd(), OsStr::new(name))
                .map_err(|error| io_error("stat search scope", workspace.root(), error))?;
            let _ = path;
            Ok(if stat.is_symlink() {
                EntryKind::Symlink
            } else if stat.is_dir() {
                EntryKind::Dir
            } else if stat.is_file() {
                EntryKind::File
            } else {
                EntryKind::Other
            })
        }
        #[cfg(not(unix))]
        {
            let _ = name;
            let target = workspace.target(path, false)?;
            let stat = std::fs::symlink_metadata(target)
                .map_err(|error| io_error("stat search scope", workspace.root(), error))?;
            Ok(if stat.is_symlink() {
                EntryKind::Symlink
            } else if stat.is_dir() {
                EntryKind::Dir
            } else if stat.is_file() {
                EntryKind::File
            } else {
                EntryKind::Other
            })
        }
    }
    #[cfg(unix)]
    pub fn file(
        &self,
        path: &str,
        name: &str,
    ) -> Result<Option<(std::fs::File, u64)>, HarnessError> {
        use super::{
            anchor::Anchor,
            fd::{open_dir_at, stat_at},
        };
        let parent = open_dir_at(self.stream.dir_fd(), OsStr::new("."))
            .map_err(|error| io_error("hold search directory", std::path::Path::new("."), error))?;
        let stat = match stat_at(parent.as_raw_fd(), OsStr::new(name)) {
            Ok(stat) => stat,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(io_error(
                    "stat search file",
                    std::path::Path::new("."),
                    error,
                ));
            }
        };
        if !stat.is_file() {
            return Err(HarnessError::NotRegularFile {
                path: String::new(),
            });
        }
        let anchor = Anchor::new(
            std::sync::Arc::new(parent),
            name.into(),
            path.into(),
            Some(stat),
        );
        let file = anchor.open_verified(stat.identity(), path)?;
        Ok(Some((file, stat.len())))
    }
    #[cfg(not(unix))]
    pub fn file(
        &self,
        relative: &str,
        _name: &str,
    ) -> Result<Option<(std::fs::File, u64)>, HarnessError> {
        let path = self.workspace.target(relative, false)?;
        let before = match std::fs::symlink_metadata(&path) {
            Ok(before) => before,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_error("stat search file", &path, error)),
        };
        if before.file_type().is_symlink() || !before.is_file() {
            return Err(HarnessError::NotRegularFile {
                path: String::new(),
            });
        }
        // The same verified handle owner as ordinary non-Unix Workspace::read.
        let file = self.workspace.open_verified(&path, &before, relative)?;
        Ok(Some((file, before.len())))
    }
    pub fn next(&mut self) -> Result<Option<ListEntry>, HarnessError> {
        #[cfg(unix)]
        {
            let Some((name, _)) = self.stream.next().map_err(|error| {
                io_error("read search directory", std::path::Path::new("."), error)
            })?
            else {
                return Ok(None);
            };
            let stat = super::fd::stat_at(self.stream.dir_fd(), &name)
                .map_err(|error| io_error("stat search entry", std::path::Path::new("."), error))?;
            let name = name.into_string().map_err(|_| HarnessError::InvalidPath {
                path: String::new(),
            })?;
            let kind = if stat.is_symlink() {
                EntryKind::Symlink
            } else if stat.is_dir() {
                EntryKind::Dir
            } else if stat.is_file() {
                EntryKind::File
            } else {
                EntryKind::Other
            };
            Ok(Some(ListEntry {
                name,
                kind,
                size: stat.len(),
                mtime: stat.modified(),
            }))
        }
        #[cfg(not(unix))]
        {
            let Some(entry) = self.stream.next() else {
                return Ok(None);
            };
            let entry = entry
                .map_err(|error| io_error("read search entry", std::path::Path::new("."), error))?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| HarnessError::InvalidPath {
                    path: String::new(),
                })?;
            let meta = std::fs::symlink_metadata(entry.path())
                .map_err(|error| io_error("stat search entry", std::path::Path::new("."), error))?;
            let kind = if meta.is_symlink() {
                EntryKind::Symlink
            } else if meta.is_dir() {
                EntryKind::Dir
            } else if meta.is_file() {
                EntryKind::File
            } else {
                EntryKind::Other
            };
            Ok(Some(ListEntry {
                name,
                kind,
                size: meta.len(),
                mtime: meta.modified().ok(),
            }))
        }
    }
}
