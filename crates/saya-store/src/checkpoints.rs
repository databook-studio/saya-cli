//! Opt-in private recovery storage for DIRECT FILE-TOOL EDITS: the exact
//! before-image bytes of one edit plus its checkpoint metadata, under the
//! per-session state directory `<sessions>/<id>/checkpoints/` (0700; files
//! 0600) so deleting the session removes them with it. These files are
//! PRIVATE RECOVERY DATA — separate from session narrative and database
//! evidence, never redacted (a faithful restore needs the exact bytes),
//! never granted any authority by existing, and never advertised or
//! packaged elsewhere. Restore stays conditional on the file's current
//! digest matching the recorded post-edit version (a later packet);
//! downgrade to older checkpoint layouts is unsupported.
//!
//! The lifecycle: [`FsSessionStore::prepare_checkpoint`] gates on policy,
//! session state, screening, and quota BEFORE any write, then stores the
//! bytes first and the `Prepared` record second — a snapshot failure never
//! masquerades as a checkpoint. At quota a recovery-required edit is
//! refused and history is never automatically evicted.
//! [`FsSessionStore::commit_checkpoint`] records the landed post-edit
//! digest; [`FsSessionStore::mark_checkpoint_uncertain`] records an unknown
//! outcome; [`FsSessionStore::recover_checkpoints`] makes that conversion
//! durable after a crash and clears orphans.

pub use screen::ScreenRefusal;

mod files;
mod ops;
mod records;
mod recover;
mod scan;
mod screen;

use crate::filesystem::is_valid_id;
use crate::session_lifecycle::tombstone;
use crate::{FsSessionStore, StoreError};
use saya_types::{CheckpointRefusal, EditSource};
use thiserror::Error;

/// Why a checkpoint operation failed. Payload-free like [`StoreError`]:
/// no path, id, or content is ever echoed back.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum CheckpointStoreError {
    /// The policy switch is off: no recovery data is written, not even the
    /// checkpoints directory.
    #[error("checkpoint recovery is disabled by policy")]
    Disabled,
    /// Admission screening refused the content before anything was stored.
    #[error("the edit content was refused by admission screening")]
    Screened(#[from] ScreenRefusal),
    /// The edit exceeds a session quota: refused before any write, never
    /// resolved by evicting existing checkpoints.
    #[error("the edit exceeds the checkpoint quota")]
    Refused(#[from] CheckpointRefusal),
    /// The underlying store failed: I/O, a pending-deletion tombstone, or
    /// an invalid session id.
    #[error("local checkpoint storage is unavailable")]
    Store(#[from] StoreError),
    /// No such checkpoint — an id that does not parse, a record that is
    /// not stored, or an `Absent` before-image when read.
    #[error("the checkpoint record does not exist")]
    NotFound,
    /// A stored record or image is unusable: unparsable, oversize,
    /// name-mismatched, a symlink, or bytes failing their digest and size
    /// verification.
    #[error("the checkpoint record or image is corrupt")]
    Corrupt,
}

/// One edit's checkpoint request: the workspace-relative path the edit
/// targets, the direct file-tool source, and the exact before-image bytes
/// when the file existed (`None` records a create with an `Absent`
/// before-image).
#[derive(Debug, Clone)]
pub struct NewCheckpoint<'a> {
    /// The edited file, relative to the workspace root.
    pub path: &'a str,
    /// The direct file-tool source of the edit.
    pub source: EditSource,
    /// The exact bytes the file had before the edit, never redacted;
    /// `None` when the file did not exist.
    pub before: Option<&'a [u8]>,
}

/// What one crash-recovery pass changed: how many `Prepared` records
/// became `Uncertain`, how many orphan before-images and staging temps
/// were removed, and how many foreign or invalid entries were left
/// untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RecoveryReport {
    /// `Prepared` records converted to `Uncertain`.
    pub marked_uncertain: usize,
    /// Orphan before-images and staging temps removed.
    pub orphans_removed: usize,
    /// Foreign or invalid entries left alone.
    pub skipped: usize,
}

/// The gate shared by every mutating checkpoint operation: the session id
/// must be valid and the session must not be tombstoned — a session being
/// deleted never gains or changes checkpoints.
pub(super) fn gated_session(
    store: &FsSessionStore,
    session_id: &str,
) -> Result<(), CheckpointStoreError> {
    if !is_valid_id(session_id) {
        return Err(StoreError::Invalid.into());
    }
    if tombstone::exists(store.sessions_root(), session_id)? {
        return Err(StoreError::Conflict.into());
    }
    Ok(())
}

/// The store-level "value is not valid for storage" refusal, as a
/// checkpoint error.
pub(super) fn invalid() -> CheckpointStoreError {
    StoreError::Invalid.into()
}
