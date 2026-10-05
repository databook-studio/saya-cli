//! Explicit project preferences: short, user-confirmed guidance text scoped
//! to one canonical worktree, with stable ids for update and forget.
//!
//! This is NOT typed database knowledge — `contract::preference` keeps that,
//! with its deliberately prose-free `PreferenceValue` — and it is NOT an
//! execution grant: no field carries a command, permission, or capability,
//! and `deny_unknown_fields` leaves no room to smuggle an authority field
//! in. The only provenance is [`PreferenceSource::UserExplicit`]:
//! assistant-inferred text must never be stored as a project preference, and
//! this packet encodes that by having no other variant. An id or worktree
//! digest is identity, not authenticity. Preferences are stored separately
//! from typed database knowledge. Pure contract — no I/O, no persistence.

mod error;
mod validate;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use error::PreferenceError;
pub use validate::validate_set;
use validate::{is_preference_id, is_worktree_id};

/// The only project-preference version this contract reads: anything else is
/// refused, never downgraded.
pub const PROJECT_PREFERENCE_VERSION: u32 = 1;
/// At most this many preferences per canonical worktree.
pub const MAX_PROJECT_PREFERENCES: usize = 64;
/// A preference's text is at most this many bytes, counted in UTF-8 bytes.
pub const MAX_PREFERENCE_TEXT_BYTES: usize = 1024;
/// The aggregate recall text for one worktree's preferences is at most this
/// many bytes.
pub const MAX_PREFERENCE_RECALL_BYTES: usize = 16 * 1024;
/// Serialized bound: the whole record is at most this many bytes of JSON.
pub const MAX_PREFERENCE_RECORD_BYTES: usize = 4 * 1024;

/// A canonical worktree identity: `wt-` followed by 64 lowercase hex digits —
/// the SHA-256 of the canonical worktree path string. The contract never
/// touches the filesystem: the caller supplies an already-canonical path.
/// A digest is identity, not authenticity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorktreeId(String);

impl WorktreeId {
    /// Hashes an already-canonical worktree path into its identity. Empty and
    /// NUL-carrying paths are refused; everything else hashes.
    pub fn from_canonical_path(path: &str) -> Result<Self, PreferenceError> {
        if path.is_empty() || path.bytes().any(|byte| byte == 0) {
            return Err(PreferenceError::InvalidCanonicalPath);
        }
        let mut hash = Sha256::new();
        hash.update(path.as_bytes());
        let hex: String = hash
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Ok(Self(format!("wt-{hex}")))
    }

    /// Parses the exact `wt-` + 64-lowercase-hex shape; anything else is
    /// refused.
    pub fn parse(value: &str) -> Result<Self, PreferenceError> {
        if !is_worktree_id(value) {
            return Err(PreferenceError::InvalidWorktree);
        }
        Ok(Self(value.to_owned()))
    }

    /// The identifier as it is serialized.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A preference identifier: `pp-` followed by 32 lowercase hex characters,
/// stable across edits so a preference can be updated or forgotten.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PreferenceId(String);

impl PreferenceId {
    /// Parses the exact `pp-` + 32-lowercase-hex shape; anything else is
    /// refused.
    pub fn parse(value: &str) -> Result<Self, PreferenceError> {
        if !is_preference_id(value) {
            return Err(PreferenceError::InvalidId);
        }
        Ok(Self(value.to_owned()))
    }

    /// The identifier as it is serialized.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// How a preference came to exist. There is exactly one source: the user said
/// so, explicitly. Assistant-inferred text must never be stored as a project
/// preference, and this packet encodes that by having no other variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PreferenceSource {
    /// The user explicitly confirmed this text.
    UserExplicit,
}

/// One explicit project preference, version 1. Loading runs the byte cap, a
/// lenient version probe, a strict parse, and [`Self::validate`] — no path
/// skips a gate. The record binds no authority: it grants no permission and
/// never changes approvals or sandbox rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectPreference {
    pub version: u32,
    pub id: PreferenceId,
    pub worktree: WorktreeId,
    pub text: String,
    pub source: PreferenceSource,
    pub created_unix_ms: i64,
    pub updated_unix_ms: i64,
}

impl ProjectPreference {
    /// Returns the record with `new_text`, keeping the same id and the same
    /// worktree — an edit can never rebind another worktree — with
    /// `updated = max(now, created)`. The new text runs the same validation
    /// as any other text.
    pub fn edited(&self, new_text: &str, now_unix_ms: i64) -> Result<Self, PreferenceError> {
        validate::validate_text(new_text)?;
        Ok(Self {
            version: self.version,
            id: self.id.clone(),
            worktree: self.worktree.clone(),
            text: new_text.to_owned(),
            source: self.source,
            created_unix_ms: self.created_unix_ms,
            updated_unix_ms: now_unix_ms.max(self.created_unix_ms),
        })
    }
}
