//! The identity of an in-place root: one canonical directory and a
//! domain-separated content address.
//!
//! The id names the canonical path only. It is an identity, never an
//! authority: every filesystem action still goes through the contained
//! [`Workspace`](crate::workspace::Workspace), and the root is re-verified
//! against its canonical path before use.

use std::{
    fmt, fs,
    path::{Path, PathBuf},
};

use saya_types::RepositoryId;
use sha2::{Digest, Sha256};
use thiserror::Error;

pub(crate) const LOCAL_ROOT_ID_DOMAIN: &[u8] = b"saya-coding-local-root-v1\0";
const MAX_ROOT_PATH_BYTES: usize = 4096;

/// The identity of one in-place folder: `loc-<sha256(domain || canonical
/// path)>`. It is a [`RepositoryId`] so the durable job row and the snapshot
/// manifest carry it without a store migration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LocalRootId(RepositoryId);

impl LocalRootId {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn as_repository_id(&self) -> &RepositoryId {
        &self.0
    }
}

impl fmt::Display for LocalRootId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One canonical in-place folder. Constructed only by [`LocalRoot::open`],
/// which refuses a file, a symlink, a missing path, and any overlap with
/// saya's state root.
pub struct LocalRoot {
    id: LocalRootId,
    path: PathBuf,
}

impl LocalRoot {
    /// Resolves `path` to its canonical directory and refuses every shape
    /// that cannot be an in-place root: a missing path, a file, a symlink,
    /// or a folder that contains the state root or sits inside it.
    pub fn open(path: &Path, state_root: &Path) -> Result<Self, LocalRootError> {
        let metadata = fs::symlink_metadata(path).map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => LocalRootError::NotFound,
            _ => LocalRootError::Io,
        })?;
        if metadata.file_type().is_symlink() {
            return Err(LocalRootError::Symlink);
        }
        if !metadata.is_dir() {
            return Err(LocalRootError::NotDirectory);
        }
        let canonical = fs::canonicalize(path).map_err(|_| LocalRootError::Io)?;
        let state = fs::canonicalize(state_root).map_err(|_| LocalRootError::Io)?;
        if state.starts_with(&canonical) || canonical.starts_with(&state) {
            return Err(LocalRootError::StateOverlap);
        }
        let id = identity(&canonical)?;
        Ok(Self {
            id,
            path: canonical,
        })
    }

    pub fn id(&self) -> &LocalRootId {
        &self.id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn identity(canonical: &Path) -> Result<LocalRootId, LocalRootError> {
    let text = canonical.to_str().ok_or(LocalRootError::UnsupportedPath)?;
    if text.len() > MAX_ROOT_PATH_BYTES {
        return Err(LocalRootError::UnsupportedPath);
    }
    let mut hasher = Sha256::new();
    hasher.update(LOCAL_ROOT_ID_DOMAIN);
    hasher.update(text.as_bytes());
    let id = format!("loc-{}", lower_hex(&hasher.finalize()));
    let id = RepositoryId::parse(&id).map_err(|_| LocalRootError::UnsupportedPath)?;
    Ok(LocalRootId(id))
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum LocalRootError {
    #[error("the in-place folder does not exist")]
    NotFound,
    #[error("the in-place path is a symlink")]
    Symlink,
    #[error("the in-place path is not a directory")]
    NotDirectory,
    #[error("the in-place folder overlaps saya's state directory")]
    StateOverlap,
    #[error("the in-place folder path cannot be identified")]
    UnsupportedPath,
    #[error("the in-place folder could not be read")]
    Io,
}
