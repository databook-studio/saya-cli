//! The containment seam: path-argument validation, component-wise symlink
//! refusal, the anchored (dirfd) component walk on unix with a
//! canonicalise-then-prefix check elsewhere, no-follow opens with a post-open
//! identity check, and atomic writes with mode discipline.

#[cfg(not(unix))]
use std::io::{Read, Write};
#[cfg(not(unix))]
use std::process;
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::fd::OwnedFd;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
#[cfg(unix)]
use std::sync::Arc;

use crate::{HarnessError, io_error};

/// A run workspace: the directory the engine created for the run's
/// artifacts. The root is canonical here — resolved once, never re-inferred —
/// and on unix pinned by a descriptor opened once, so every later walk
/// starts from the root the engine actually created.
#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
    #[cfg(unix)]
    root_dir_fd: Arc<OwnedFd>,
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
    /// Lowercase hex sha256 of the file's whole bytes — streamed in one
    /// open, so the digest never costs the model a second read and never
    /// serves content beyond the bound.
    pub digest: String,
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
    /// The root is canonicalised once here and pinned by a descriptor (unix),
    /// so every later check and every later walk starts from the directory
    /// this call actually opened.
    pub fn open(root: &Path) -> Result<Self, HarnessError> {
        let canonical = fs::canonicalize(root)
            .map_err(|error| io_error("resolve workspace root", root, error))?;
        if !canonical.is_dir() {
            return Err(HarnessError::NotRegularFile {
                path: root.display().to_string(),
            });
        }
        #[cfg(unix)]
        let root_dir_fd = Arc::new(
            super::fd::open_root_fd(canonical.as_os_str())
                .map_err(|error| io_error("open workspace root", root, error))?,
        );
        Ok(Self {
            root: canonical,
            #[cfg(unix)]
            root_dir_fd,
        })
    }

    /// The canonical workspace root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The pinned root descriptor the anchored walks start from (unix).
    #[cfg(unix)]
    pub(crate) fn root_dir_fd(&self) -> &Arc<OwnedFd> {
        &self.root_dir_fd
    }

    /// Reads a workspace file under containment: validation, a no-follow
    /// open, and a post-open identity check so the bytes served belong to
    /// the file that was scanned. At most `max_bytes` are returned, with
    /// `truncated` set when the file holds more; the digest covers the whole
    /// file — hashed in the same open, streamed in chunks — so a truncated
    /// read still names the state an edit precondition can state.
    pub fn read(&self, rel: &str, max_bytes: u64) -> Result<ReadFile, HarnessError> {
        #[cfg(unix)]
        return super::anchored::read(self, rel, max_bytes);

        #[cfg(not(unix))]
        {
            use sha2::{Digest, Sha256};

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
            let mut hasher = Sha256::new();
            let mut chunk = [0u8; 64 * 1024];
            let mut kept = 0u64;
            loop {
                use std::io::Read as _;

                let read = (&file)
                    .read(&mut chunk)
                    .map_err(|error| io_error("read workspace file", &path, error))?;
                if read == 0 {
                    break;
                }
                hasher.update(&chunk[..read]);
                let room = max_bytes.saturating_sub(kept) as usize;
                let take = read.min(room);
                bytes.extend_from_slice(&chunk[..take]);
                kept += take as u64;
            }
            let digest: String = hasher
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            Ok(ReadFile {
                bytes,
                size: pre.len(),
                truncated: pre.len() > max_bytes,
                digest,
            })
        }
    }

    /// Writes a workspace file atomically: temp file at 0600, fsync, rename
    /// over the target. The rename replaces whatever the path names without
    /// following it, so content cannot land outside the root; the path is
    /// re-verified afterwards so a post-write swap is reported rather than
    /// pretended away. No execute bits are ever set.
    pub fn write(&self, rel: &str, bytes: &[u8]) -> Result<(), HarnessError> {
        #[cfg(unix)]
        return super::anchored::write(self, rel, bytes);

        #[cfg(not(unix))]
        {
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
            replace_file(&temp_path, &path)?;
            let final_meta = fs::symlink_metadata(&path)
                .map_err(|error| io_error("verify written workspace file", &path, error))?;
            let exec_bits = false;
            if identity(&final_meta) != written || !final_meta.is_file() || exec_bits {
                return Err(HarnessError::IdentityChanged {
                    path: rel.to_string(),
                });
            }
            Ok(())
        }
    }

    /// Lists a workspace directory, bounded by `max_entries`. The empty
    /// argument names the workspace root itself.
    pub fn list(&self, rel: &str, max_entries: usize) -> Result<Vec<ListEntry>, HarnessError> {
        #[cfg(unix)]
        return super::anchored::list(self, rel, max_entries);

        #[cfg(not(unix))]
        {
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
                let entry =
                    entry.map_err(|error| io_error("list workspace directory", &dir, error))?;
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
                // Checked after the push, so the refusal fires when a directory
                // holds MORE than `max_entries` entries — the (max+1)-th entry
                // trips the bound with found > max, matching the search bounds
                // and the walk's visited bound; a quietly shortened listing would
                // read as the whole directory.
                if entries.len() > max_entries {
                    return Err(HarnessError::BoundsExceeded {
                        path: rel.to_string(),
                        found: entries.len() as u64,
                        max: max_entries as u64,
                    });
                }
            }
            entries.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(entries)
        }
    }

    /// Validates `rel` and resolves it under the canonical root: argument
    /// validation and a component walk that refuses symlinks (creating missing
    /// directories only when `allow_create`). On unix this is the anchored
    /// walk — each component resolved against the previous descriptor — and
    /// the returned path is the walk's own record; elsewhere it is the
    /// component-by-path walk plus the canonicalise-then-prefix check.
    /// `pub(crate)` so the contained walk (`walk`) can re-run the exact same
    /// check on every candidate it reports — the one validator, reused, not
    /// re-derived.
    pub(crate) fn target(&self, rel: &str, allow_create: bool) -> Result<PathBuf, HarnessError> {
        #[cfg(unix)]
        return Ok(self.anchor(rel, allow_create)?.path().to_path_buf());

        #[cfg(not(unix))]
        {
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
    }

    /// Canonicalise-then-prefix: the deepest anchor that exists — the path
    /// itself, or its parent when the final component is still to be
    /// created — must resolve under the canonical root. The unix walk is
    /// anchored by construction (each component resolved against the previous
    /// descriptor); this is the non-unix belt.
    #[cfg(not(unix))]
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
    /// adopted. The non-unix shape; the unix write anchors the temp to the
    /// walked parent descriptor instead. Shared with the range patch's
    /// non-unix commit path.
    #[cfg(not(unix))]
    pub(crate) fn create_temp(
        &self,
        parent: &Path,
        rel: &str,
    ) -> Result<(PathBuf, fs::File), HarnessError> {
        for attempt in 0..8 {
            let name = format!(".saya-tmp-{}-{}-{attempt}", process::id(), nanos());
            let candidate = parent.join(name);
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
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

    /// Opens the download machinery's partial file under containment: the same
    /// argument validation and component walk as any workspace write,
    /// then a no-follow open at 0600 — `truncate` for a fresh download,
    /// append for a resume — with a post-open identity check so the file
    /// streamed into is the one that was scanned. Never executable.
    pub(crate) fn open_download_part(
        &self,
        rel: &str,
        truncate: bool,
    ) -> Result<(PathBuf, fs::File), HarnessError> {
        #[cfg(unix)]
        return super::download::open_download_part(self, rel, truncate);

        #[cfg(not(unix))]
        {
            let path = self.target(rel, true)?;
            let mut options = OpenOptions::new();
            options.write(true).create(true);
            if truncate {
                options.truncate(true);
            } else {
                options.append(true);
            }
            let file = options
                .open(&path)
                .map_err(|error| io_error("open workspace download part", &path, error))?;
            let opened = file
                .metadata()
                .map_err(|error| io_error("stat opened download part", &path, error))?;
            let current = fs::symlink_metadata(&path)
                .map_err(|error| io_error("verify opened download part", &path, error))?;
            if identity(&current) != identity(&opened) {
                return Err(HarnessError::IdentityChanged {
                    path: rel.to_string(),
                });
            }
            Ok((path, file))
        }
    }

    /// Promotes a completed download part over `dest_rel` atomically: the
    /// rename replaces whatever the path names without following it, and the
    /// destination is re-verified afterwards (regular file, 0600, never
    /// executable, still the inode that was renamed) so a completed download
    /// is never pretended onto a swapped path. Both part and destination are
    /// anchored to the walked descriptors, so the rename acts on the
    /// directories the walk validated.
    pub(crate) fn promote_download(
        &self,
        part_rel: &str,
        dest_rel: &str,
    ) -> Result<(), HarnessError> {
        #[cfg(unix)]
        return super::download::promote(self, part_rel, dest_rel);

        #[cfg(not(unix))]
        {
            let dest = self.target(dest_rel, true)?;
            let part_identity = {
                let meta = fs::symlink_metadata(part_rel)
                    .map_err(|error| io_error("stat download part", dest.as_path(), error))?;
                if !meta.is_file() {
                    return Err(HarnessError::NotRegularFile {
                        path: dest.display().to_string(),
                    });
                }
                identity(&meta)
            };
            let part = self.root.join(part_rel);
            replace_file(&part, &dest)?;
            let final_meta = fs::symlink_metadata(&dest)
                .map_err(|error| io_error("verify written workspace file", &dest, error))?;
            let exec_bits = false;
            if identity(&final_meta) != part_identity || !final_meta.is_file() || exec_bits {
                return Err(HarnessError::IdentityChanged {
                    path: dest_rel.to_string(),
                });
            }
            Ok(())
        }
    }

    /// Removes a workspace file relative to its validated parent; absence is
    /// already-done, not an error. The sidecar removal's contained form.
    pub(crate) fn unlink(&self, rel: &str) -> Result<(), HarnessError> {
        #[cfg(unix)]
        return super::download::unlink(self, rel);

        #[cfg(not(unix))]
        {
            let path = self.target(rel, false)?;
            match fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(io_error("remove download sidecar", &path, error)),
            }
        }
    }
}

/// The argument half of [`Workspace::target`] with no filesystem contact:
/// argument validation plus the `..`-depth walk, assuming every non-`..`
/// component is a directory — the conservative shape check that refuses an
/// escape (absolute forms, `..` rising above the root, drive shapes) before
/// any scan or creation can happen. A caller that must refuse a path before
/// touching the disk runs this first and then `target`.
pub(crate) fn validate_argument(rel: &str) -> Result<(), HarnessError> {
    let comps = argument_components(rel)?;
    let mut depth = 0usize;
    for comp in comps {
        if comp == ".." {
            if depth == 0 {
                return Err(HarnessError::PathOutsideRoot {
                    path: rel.to_string(),
                });
            }
            depth -= 1;
        } else {
            depth += 1;
        }
    }
    Ok(())
}

pub(crate) fn argument_components(rel: &str) -> Result<Vec<String>, HarnessError> {
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

#[cfg(not(unix))]
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
pub(crate) fn identity(metadata: &fs::Metadata) -> (u64, u64) {
    (metadata.dev(), metadata.ino())
}

/// File identity for the non-unix commit paths: (length, creation time),
/// the weaker stated posture — length stands in where unix has (dev, inode).
/// Shared with the range patch's non-unix commit path.
#[cfg(not(unix))]
pub(crate) fn identity_of(metadata: &fs::Metadata) -> (u64, u64) {
    use std::os::windows::fs::MetadataExt as _;
    #[cfg(windows)]
    {
        (metadata.len(), metadata.creation_time())
    }
    #[cfg(not(windows))]
    {
        let _ = metadata;
        (metadata.len(), 0)
    }
}

/// Atomic replace for the non-unix commit paths, shared with the range
/// patch: rename on unix-shaped targets, copy+remove on Windows.
#[cfg(not(unix))]
pub(crate) fn replace_workspace_file(temp: &Path, target: &Path) -> Result<(), HarnessError> {
    replace_file(temp, target)
}

#[cfg(not(unix))]
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

pub(crate) fn nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or_default()
}
