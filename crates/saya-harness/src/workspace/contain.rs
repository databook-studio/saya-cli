//! The containment seam: path-argument validation, component-wise symlink
//! refusal, the anchored (dirfd) component walk on unix with a
//! canonicalise-then-prefix check elsewhere, no-follow opens with a post-open
//! identity check, and atomic writes with mode discipline.

#[cfg(not(unix))]
use std::io::Write;
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

/// Opens a reparse point itself so a final-component link cannot be followed
/// between the path scan and a Windows handle verification.
#[cfg(windows)]
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

/// Hard ceiling for ordinary workspace byte operations. Public methods still
/// accept operation-specific bounds; scratch CSV import has its own read-only
/// ceiling below, while ordinary reads and all writes stay at this one.
pub const MAX_IO_BYTES: usize = 8 * 1024 * 1024;
/// The separate ceiling for a single CSV import into the scratch database.
pub const MAX_SCRATCH_IMPORT_BYTES: usize = 32 * 1024 * 1024;
/// Hard ceiling for one directory enumeration. Search callers use smaller
/// limits; this prevents a direct caller from turning listing into a flood.
pub const MAX_LIST_ENTRIES: usize = 2_000;

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
}

mod download;
mod identity;
mod list;
mod path;
mod read;
mod write;

#[cfg(not(windows))]
pub(crate) use identity::identity_of;
pub(crate) use identity::nanos;
#[cfg(not(unix))]
pub(crate) use identity::{file_identity, path_identity, replace_workspace_file};
pub(crate) use path::{argument_components, validate_argument};
