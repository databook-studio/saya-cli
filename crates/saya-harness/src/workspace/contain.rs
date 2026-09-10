//! The containment seam: path-argument validation, component-wise symlink
//! refusal, canonicalise-then-prefix checks, no-follow opens with a post-open
//! identity check, and atomic writes with mode discipline.

use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

use crate::{HarnessError, io_error};

/// A run workspace: the directory the engine created for the run's
/// artifacts. The root is canonical here — resolved once, never re-inferred.
#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
}

/// One file read under containment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadFile {
    /// At most the read bound of content.
    pub bytes: Vec<u8>,
    /// The file's size as scanned before opening; `bytes` may be a prefix.
    pub size: u64,
    /// Whether `bytes` was capped at the read bound.
    pub truncated: bool,
}

/// The kind of one directory entry as [`Workspace::list`] reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
    Other,
}

/// One directory entry with its name and size. Symlinks are reported as
/// [`EntryKind::Symlink`] — listed, never opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListEntry {
    pub name: String,
    pub kind: EntryKind,
    pub size: u64,
}

impl Workspace {
    /// Opens the workspace at `root`, which the engine has already created.
    /// The root is canonicalised once here; every later check compares
    /// against this canonical form.
    pub fn open(root: &Path) -> Result<Self, HarnessError> {
        let canonical = fs::canonicalize(root)
            .map_err(|error| io_error("resolve workspace root", root, error))?;
        if !canonical.is_dir() {
            return Err(HarnessError::NotRegularFile {
                path: root.display().to_string(),
            });
        }
        Ok(Self { root: canonical })
    }

    /// The canonical workspace root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Reads a workspace file under containment: validation, a no-follow
    /// open, and a post-open identity check so the bytes served belong to
    /// the file that was scanned. At most `max_bytes` are returned, with
    /// `truncated` set when the file holds more.
    pub fn read(&self, rel: &str, max_bytes: u64) -> Result<ReadFile, HarnessError> {
        let path = self.target(rel, false)?;
        let pre = fs::symlink_metadata(&path)
            .map_err(|error| io_error("read workspace file", &path, error))?;
        if pre.file_type().is_symlink() {
            return Err(HarnessError::SymlinkRefused {
                path: rel.to_string(),
            });
        }
        if !pre.is_file() {
            return Err(HarnessError::NotRegularFile {
                path: rel.to_string(),
            });
        }
        let file = self.open_verified(&path, &pre, rel)?;
        let mut bytes = Vec::new();
        (&file)
            .take(max_bytes.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| io_error("read workspace file", &path, error))?;
        let truncated = bytes.len() as u64 > max_bytes;
        if truncated {
            bytes.truncate(max_bytes as usize);
        }
        Ok(ReadFile {
            bytes,
            size: pre.len(),
            truncated,
        })
    }

    /// Writes a workspace file atomically: temp file at 0600, fsync, rename
    /// over the target. The rename replaces whatever the path names without
    /// following it, so content cannot land outside the root; the path is
    /// re-verified afterwards so a post-write swap is reported rather than
    /// pretended away. No execute bits are ever set.
    pub fn write(&self, rel: &str, bytes: &[u8]) -> Result<(), HarnessError> {
        let path = self.target(rel, true)?;
        if let Ok(meta) = fs::symlink_metadata(&path) {
            if meta.file_type().is_symlink() {
                return Err(HarnessError::SymlinkRefused {
                    path: rel.to_string(),
                });
            }
            if !meta.is_file() {
                return Err(HarnessError::NotRegularFile {
                    path: rel.to_string(),
                });
            }
        }
        let parent = path.parent().unwrap_or(self.root.as_path()).to_path_buf();
        let (temp_path, mut temp) = self.create_temp(&parent, rel)?;
        temp.write_all(bytes)
            .map_err(|error| io_error("write workspace temp", &temp_path, error))?;
        temp.sync_all()
            .map_err(|error| io_error("sync workspace temp", &temp_path, error))?;
        let written = identity(
            &temp
                .metadata()
                .map_err(|error| io_error("stat workspace temp", &temp_path, error))?,
        );
        drop(temp);
        #[cfg(unix)]
        set_mode(&temp_path, 0o600)?;
        replace_file(&temp_path, &path)?;
        let final_meta = fs::symlink_metadata(&path)
            .map_err(|error| io_error("verify written workspace file", &path, error))?;
        #[cfg(unix)]
        let exec_bits = final_meta.permissions().mode() & 0o111 != 0;
        #[cfg(not(unix))]
        let exec_bits = false;
        if identity(&final_meta) != written || !final_meta.is_file() || exec_bits {
            return Err(HarnessError::IdentityChanged {
                path: rel.to_string(),
            });
        }
        Ok(())
    }

    /// Lists a workspace directory, bounded by `max_entries`. The empty
    /// argument names the workspace root itself.
    pub fn list(&self, rel: &str, max_entries: usize) -> Result<Vec<ListEntry>, HarnessError> {
        let dir = if rel.is_empty() {
            self.root.clone()
        } else {
            let path = self.target(rel, false)?;
            let meta = fs::symlink_metadata(&path)
                .map_err(|error| io_error("list workspace directory", &path, error))?;
            if meta.file_type().is_symlink() {
                return Err(HarnessError::SymlinkRefused {
                    path: rel.to_string(),
                });
            }
            if !meta.is_dir() {
                return Err(HarnessError::NotRegularFile {
                    path: rel.to_string(),
                });
            }
            path
        };
        let read = fs::read_dir(&dir)
            .map_err(|error| io_error("list workspace directory", &dir, error))?;
        let mut entries = Vec::new();
        for entry in read {
            if entries.len() > max_entries {
                return Err(HarnessError::BoundsExceeded {
                    path: rel.to_string(),
                    found: entries.len() as u64,
                    max: max_entries as u64,
                });
            }
            let entry = entry.map_err(|error| io_error("list workspace directory", &dir, error))?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| HarnessError::Io {
                    context: format!("workspace entry name is not UTF-8 in {}", dir.display()),
                    source: std::io::Error::new(std::io::ErrorKind::InvalidData, "entry name"),
                })?;
            let file_type = entry
                .file_type()
                .map_err(|error| io_error("list workspace directory", &dir, error))?;
            let kind = if file_type.is_symlink() {
                EntryKind::Symlink
            } else if file_type.is_dir() {
                EntryKind::Dir
            } else if file_type.is_file() {
                EntryKind::File
            } else {
                EntryKind::Other
            };
            let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
            entries.push(ListEntry { name, kind, size });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    /// Validates `rel` and resolves it under the canonical root: argument
    /// validation, a component walk that refuses symlinks (creating missing
    /// directories only when `allow_create`), then the prefix check.
    /// `pub(crate)` so the contained walk (`walk`) can re-run the exact same
    /// check on every candidate it reports — the one validator, reused, not
    /// re-derived.
    pub(crate) fn target(&self, rel: &str, allow_create: bool) -> Result<PathBuf, HarnessError> {
        let comps = argument_components(rel)?;
        let mut path = self.root.clone();
        let mut depth = 0usize;
        let mut final_exists = true;
        for (index, comp) in comps.iter().enumerate() {
            let last = index + 1 == comps.len();
            if comp == ".." {
                if depth == 0 {
                    return Err(HarnessError::PathOutsideRoot {
                        path: rel.to_string(),
                    });
                }
                depth -= 1;
                path.pop();
                continue;
            }
            path.push(comp);
            match fs::symlink_metadata(&path) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Err(HarnessError::SymlinkRefused {
                        path: rel.to_string(),
                    });
                }
                Ok(meta) => {
                    if !last {
                        if !meta.is_dir() {
                            return Err(HarnessError::InvalidPath {
                                path: rel.to_string(),
                            });
                        }
                        depth += 1;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if !last {
                        if !allow_create {
                            return Err(io_error("scan workspace path", &path, error));
                        }
                        create_dir_component(&path, rel)?;
                        depth += 1;
                    } else {
                        final_exists = false;
                    }
                }
                Err(error) => return Err(io_error("scan workspace path", &path, error)),
            }
        }
        self.prefix_check(&path, rel, final_exists)?;
        Ok(path)
    }

    /// Canonicalise-then-prefix: the deepest anchor that exists — the path
    /// itself, or its parent when the final component is still to be
    /// created — must resolve under the canonical root.
    fn prefix_check(&self, path: &Path, rel: &str, exists: bool) -> Result<(), HarnessError> {
        let anchor = if exists {
            path.to_path_buf()
        } else {
            path.parent().unwrap_or(self.root.as_path()).to_path_buf()
        };
        let canonical = fs::canonicalize(&anchor)
            .map_err(|error| io_error("canonicalise workspace path", &anchor, error))?;
        if !canonical.starts_with(&self.root) {
            return Err(HarnessError::PathOutsideRoot {
                path: rel.to_string(),
            });
        }
        Ok(())
    }

    /// Opens a fresh temp file at 0600 inside the validated parent with
    /// `O_CREAT|O_EXCL`, so a pre-planted name (even a link) can never be
    /// adopted.
    fn create_temp(&self, parent: &Path, rel: &str) -> Result<(PathBuf, fs::File), HarnessError> {
        for attempt in 0..8 {
            let name = format!(".saya-tmp-{}-{}-{attempt}", process::id(), nanos());
            let candidate = parent.join(name);
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                options.mode(0o600).custom_flags(libc::O_CLOEXEC);
            }
            match options.open(&candidate) {
                Ok(file) => return Ok((candidate, file)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(io_error("create workspace temp", &candidate, error)),
            }
        }
        Err(HarnessError::Io {
            context: format!("name a unique workspace temp file for {rel}"),
            source: std::io::Error::new(std::io::ErrorKind::AlreadyExists, "temp name collisions"),
        })
    }

    /// Opens `path` for reading after the pre-open scan `pre`: the open
    /// itself is no-follow (a link swapped into the final component fails
    /// the open), and the opened file's (dev, inode) must match the scan.
    pub(crate) fn open_verified(
        &self,
        path: &Path,
        pre: &fs::Metadata,
        rel: &str,
    ) -> Result<fs::File, HarnessError> {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        let file = options.open(path).map_err(|error| {
            #[cfg(unix)]
            if error.raw_os_error() == Some(libc::ELOOP) {
                return HarnessError::SymlinkRefused {
                    path: rel.to_string(),
                };
            }
            io_error("open workspace file", path, error)
        })?;
        let opened = file
            .metadata()
            .map_err(|error| io_error("stat opened workspace file", path, error))?;
        if identity(&opened) != identity(pre) {
            return Err(HarnessError::IdentityChanged {
                path: rel.to_string(),
            });
        }
        Ok(file)
    }
}

fn argument_components(rel: &str) -> Result<Vec<String>, HarnessError> {
    let invalid = || HarnessError::InvalidPath {
        path: rel.to_string(),
    };
    let escaped = || HarnessError::PathOutsideRoot {
        path: rel.to_string(),
    };
    if rel.is_empty() || rel.as_bytes().contains(&0) || rel.contains('\\') {
        return Err(invalid());
    }
    if rel.starts_with('/') {
        return Err(escaped());
    }
    let comps: Vec<String> = rel.split('/').map(str::to_string).collect();
    if comps.iter().any(String::is_empty) || comps.iter().any(|comp| comp == ".") {
        return Err(invalid());
    }
    if comps[0].len() == 2 && comps[0].ends_with(':') {
        return Err(invalid());
    }
    if comps.iter().any(|comp| comp.eq_ignore_ascii_case(".git")) {
        return Err(HarnessError::DeniedName {
            path: rel.to_string(),
        });
    }
    Ok(comps)
}

fn create_dir_component(path: &Path, rel: &str) -> Result<(), HarnessError> {
    match fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(_) => {
            // Lost a creation race; the winner must be a real directory.
            let meta = fs::symlink_metadata(path)
                .map_err(|error| io_error("scan workspace path", path, error))?;
            if meta.file_type().is_symlink() {
                return Err(HarnessError::SymlinkRefused {
                    path: rel.to_string(),
                });
            }
            if !meta.is_dir() {
                return Err(HarnessError::InvalidPath {
                    path: rel.to_string(),
                });
            }
            Ok(())
        }
    }
}

#[cfg(unix)]
fn identity(metadata: &fs::Metadata) -> (u64, u64) {
    (metadata.dev(), metadata.ino())
}

#[cfg(windows)]
fn identity(metadata: &fs::Metadata) -> (u64, u64) {
    use std::os::windows::fs::MetadataExt;
    (metadata.len(), metadata.creation_time())
}

fn replace_file(temp: &Path, target: &Path) -> Result<(), HarnessError> {
    #[cfg(windows)]
    {
        fs::copy(temp, target)
            .map_err(|error| io_error("replace workspace file", target, error))?;
        fs::remove_file(temp).map_err(|error| io_error("remove workspace temp", temp, error))
    }
    #[cfg(not(windows))]
    fs::rename(temp, target).map_err(|error| io_error("replace workspace file", target, error))
}

fn nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or_default()
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), HarnessError> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|error| io_error("set mode on", path, error))
}
