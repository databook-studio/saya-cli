use std::{
    fmt,
    path::{Path, PathBuf},
};

use saya_types::{GitObjectId, RepositoryId, Sha256Digest};
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::command::{GitError, GitObjectFormat};

const REPOSITORY_ID_DOMAIN: &[u8] = b"saya-coding-repository-v1\0";

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum GitHeadState {
    Commit(GitObjectId),
    Unborn,
}

#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum GitBranchState {
    Attached(String),
    Detached,
    Unborn(String),
}

impl fmt::Debug for GitBranchState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Attached(_) => formatter.write_str("Attached([branch])"),
            Self::Detached => formatter.write_str("Detached"),
            Self::Unborn(_) => formatter.write_str("Unborn([branch])"),
        }
    }
}

pub struct GitRepositoryIdentity {
    pub(super) repository_id: RepositoryId,
    pub(super) common_dir_sha256: Sha256Digest,
    pub(super) root: PathBuf,
    pub(super) git_dir: PathBuf,
    pub(super) common_dir: PathBuf,
    pub(super) object_format: GitObjectFormat,
}

impl GitRepositoryIdentity {
    pub fn repository_id(&self) -> &RepositoryId {
        &self.repository_id
    }

    pub fn common_dir_sha256(&self) -> &Sha256Digest {
        &self.common_dir_sha256
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    pub fn common_dir(&self) -> &Path {
        &self.common_dir
    }

    pub fn object_format(&self) -> GitObjectFormat {
        self.object_format
    }
}

impl Clone for GitRepositoryIdentity {
    fn clone(&self) -> Self {
        Self {
            repository_id: self.repository_id.clone(),
            common_dir_sha256: self.common_dir_sha256.clone(),
            root: self.root.clone(),
            git_dir: self.git_dir.clone(),
            common_dir: self.common_dir.clone(),
            object_format: self.object_format,
        }
    }
}

impl PartialEq for GitRepositoryIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.repository_id == other.repository_id
            && self.common_dir_sha256 == other.common_dir_sha256
            && self.root == other.root
            && self.git_dir == other.git_dir
            && self.common_dir == other.common_dir
            && self.object_format == other.object_format
    }
}

impl Eq for GitRepositoryIdentity {}

impl fmt::Debug for GitRepositoryIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GitRepositoryIdentity")
            .field("repository_id", &self.repository_id)
            .field("common_dir_sha256", &self.common_dir_sha256)
            .field("root", &"[repository root]")
            .field("git_dir", &"[git directory]")
            .field("common_dir", &"[common git directory]")
            .field("object_format", &self.object_format)
            .finish()
    }
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum GitInventoryError {
    #[error("Git inventory operation failed")]
    Git(#[source] GitError),
    #[error("Git inventory output is malformed or inconsistent")]
    MalformedOutput,
    #[error("Git inventory contains a path that escapes its repository root")]
    UnsafePath,
    #[error("repository identity requires a canonical UTF-8 common-directory path")]
    UnsupportedIdentity,
    #[error("repository contains an unsupported nested Git repository")]
    NestedRepository,
    #[error("repository contains an unsupported worktree state")]
    UnsupportedWorktree,
    #[error("repository metadata could not be safely inspected")]
    UnsupportedRepositoryState,
    #[error("Git inventory exceeds its bounded path, output, or metadata limit")]
    TooLarge,
}

pub(super) fn identity(
    common_dir: &Path,
) -> Result<(RepositoryId, Sha256Digest), GitInventoryError> {
    let locator = common_dir
        .to_str()
        .ok_or(GitInventoryError::UnsupportedIdentity)?
        .as_bytes();
    let common_digest = Sha256::digest(locator);
    let mut repository_hasher = Sha256::new();
    repository_hasher.update(REPOSITORY_ID_DOMAIN);
    repository_hasher.update(locator);
    let repository_id = format!("repo-{}", lower_hex(&repository_hasher.finalize()));
    let common_digest = lower_hex(&common_digest);
    let repository_id =
        RepositoryId::parse(&repository_id).map_err(|_| GitInventoryError::MalformedOutput)?;
    let common_digest =
        Sha256Digest::parse(&common_digest).map_err(|_| GitInventoryError::MalformedOutput)?;
    Ok((repository_id, common_digest))
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(*byte >> 4) as usize] as char);
        output.push(HEX[(*byte & 0x0f) as usize] as char);
    }
    output
}
