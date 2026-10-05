//! Opt-in private recovery of DIRECT FILE-TOOL EDITS: versioned checkpoint
//! metadata, digest identity, source kind, quota policy, and restore
//! eligibility.
//!
//! A checkpoint covers direct file-tool edits only (`workspace_edit` and
//! `workspace_write`); it never represents, and can never undo, a subprocess,
//! network, scratch, or production database effect. Checkpoints are private
//! recovery data, separate from session narrative and database evidence; they
//! grant no authority, and a stored digest detects drift but does not
//! authenticate. Pure contract — no I/O, no persistence, no byte retention:
//! the before-image bytes live in a later storage packet.

mod error;
mod policy;
mod validate;

use serde::{Deserialize, Serialize};

pub use error::{CheckpointError, CheckpointRefusal};
pub use policy::{
    CheckpointPolicy, CheckpointUsage, RestoreEligibility, admit, restore_eligibility,
};
use validate::is_checkpoint_id;

/// The only checkpoint version this contract reads: anything else is
/// refused, never downgraded.
pub const CHECKPOINT_VERSION: u32 = 1;
/// Serialized bound: the whole record is at most this many bytes of JSON.
pub const MAX_CHECKPOINT_RECORD_BYTES: usize = 4 * 1024;
/// Per-file image bound: a recorded version is at most 8 MiB, matching the
/// existing workspace IO ceiling (`MAX_IO_BYTES` in `saya-harness`'s
/// workspace containment, which this pure crate cannot import).
pub const MAX_CHECKPOINT_FILE_BYTES: u64 = 8 * 1024 * 1024;
/// Session quotas: at most these retained bytes and this many checkpoints
/// per session. At a quota a recovery-required edit is refused before
/// mutation, and history is never automatically evicted.
pub const MAX_CHECKPOINT_SESSION_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_CHECKPOINTS_PER_SESSION: u32 = 128;
/// A checkpoint path is at most this many bytes.
pub const MAX_CHECKPOINT_PATH_BYTES: usize = 512;

/// The tool that produced the edit. A checkpoint covers direct file-tool
/// edits only; no subprocess, shell, scratch, SQL, or network source exists,
/// because those effects are never checkpointed and never undoable by one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EditSource {
    WorkspaceEdit,
    WorkspaceWrite,
}

/// Lifecycle of a checkpoint: `Prepared` stores the before-image before the
/// edit is known to land; `Committed` records that the edit landed, with the
/// post-edit digest recorded; `Uncertain` records that a crash or failure
/// left the outcome unknown — never silently converted to committed or
/// discarded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CheckpointState {
    Prepared,
    Committed,
    Uncertain,
}

/// Digest identity of one recorded file version: the digest detects drift
/// and does not authenticate, and the size fits the per-file ceiling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileVersion {
    pub sha256: String,
    pub size: u64,
}

/// The before-image of an edit. `Absent` records that the file did not exist
/// before the edit (a create): restoring such an edit would be deletion,
/// which a later policy packet may allow — this contract only represents it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BeforeImage {
    Absent,
    Present(FileVersion),
}

/// A checkpoint identifier: `ck-` followed by 32 lowercase hex characters.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CheckpointId(String);

impl CheckpointId {
    /// Parses the exact `ck-` + 32-lowercase-hex shape; anything else is
    /// refused.
    pub fn parse(value: &str) -> Result<Self, CheckpointError> {
        if !is_checkpoint_id(value) {
            return Err(CheckpointError::InvalidId);
        }
        Ok(Self(value.to_owned()))
    }

    /// The identifier as it is serialized.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One direct file-tool edit and the recovery data behind it: which file,
/// through which tool, in which lifecycle state, with the before-image and,
/// once committed, the post-edit version. Loading runs the byte cap, a
/// lenient version probe, a strict parse, and [`Self::validate`] — no path
/// skips a gate. The record binds no authority: it grants nothing by
/// existing, and restore stays conditional on the current digest matching
/// the recorded post-edit version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditCheckpoint {
    pub version: u32,
    pub id: CheckpointId,
    pub session_id: String,
    /// The edited file, relative to the workspace root.
    pub path: String,
    pub source: EditSource,
    pub state: CheckpointState,
    pub before: BeforeImage,
    /// The recorded post-edit version; present exactly when committed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<FileVersion>,
    pub created_unix_ms: i64,
    pub updated_unix_ms: i64,
}
