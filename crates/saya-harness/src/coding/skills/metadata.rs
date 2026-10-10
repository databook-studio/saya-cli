use thiserror::Error;

use crate::workspace::Workspace;

use super::MAX_SKILL_LABEL_BYTES;

/// A descriptive origin for indexed skill metadata; it is not an authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillOrigin {
    User,
    Project,
}

/// A trusted caller-selected workspace and descriptive source label.
#[derive(Debug, Clone)]
pub struct SkillRoot {
    pub(super) workspace: Workspace,
    pub(super) origin: SkillOrigin,
    pub(super) trust_label: String,
}

impl SkillRoot {
    /// Binds a workspace selected by the caller to a non-authoritative label.
    pub fn new(
        workspace: Workspace,
        origin: SkillOrigin,
        trust_label: impl Into<String>,
    ) -> Result<Self, SkillIndexError> {
        let trust_label = trust_label.into();
        if trust_label.is_empty()
            || trust_label.len() > MAX_SKILL_LABEL_BYTES
            || trust_label.chars().any(char::is_control)
        {
            return Err(SkillIndexError::InvalidLabel);
        }
        Ok(Self {
            workspace,
            origin,
            trust_label,
        })
    }
}

/// One metadata-only discovery result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub origin: SkillOrigin,
    /// Caller-provided display context only; never a capability grant.
    pub trust_label: String,
    /// Index into the root slice supplied to `discover_skills` for later
    /// caller-controlled reopening; this value grants no path authority.
    pub root_index: usize,
    /// Safe, relative direct-child directory name below that root.
    pub relative_dir: String,
    /// Lowercase SHA-256 of the exact frontmatter bytes, including delimiters.
    pub metadata_sha256: String,
}

/// The deterministic metadata-only skill listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillIndex {
    pub skills: Vec<SkillEntry>,
}

/// Bounded discovery refusals. Diagnostics intentionally omit source contents
/// and candidate paths because both are untrusted input.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SkillIndexError {
    #[error("too many configured skill roots")]
    TooManyRoots,
    #[error("more than one skill root has the same origin")]
    DuplicateOrigin,
    #[error("invalid descriptive skill source label")]
    InvalidLabel,
    #[error("skill root enumeration exceeded its entry bound ({max})")]
    EntryLimit { max: usize },
    #[error("skill metadata headers exceeded the aggregate bound ({max})")]
    AggregateHeaderTooLarge { max: usize },
    #[error("skill metadata header exceeded its bound ({max})")]
    HeaderTooLarge { max: usize },
    #[error("skill metadata is malformed or unsupported")]
    MalformedFrontmatter,
    #[error("skill directory name is not a safe single path component")]
    InvalidSkillPath,
    #[error("a skill candidate is a symlink")]
    SymlinkRefused,
    #[error("a skill candidate is not a regular file")]
    NotRegularFile,
    #[error("two skills from the same origin declare the same name")]
    Conflict,
    #[error("skill workspace operation was refused")]
    WorkspaceRefused,
    #[error("skill discovery is unsupported on this platform")]
    UnsupportedPlatform,
}

#[cfg(unix)]
pub(super) struct ParsedMetadata {
    pub name: String,
    pub description: String,
}
