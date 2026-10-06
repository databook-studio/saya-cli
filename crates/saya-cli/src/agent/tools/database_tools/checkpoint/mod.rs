//! Checkpoint capture around DIRECT FILE-TOOL EDITS: the recorder the
//! model-facing file tools consult when recovery is enabled. A capture is
//! prepared BEFORE the guarded commit (a capture failure prevents the
//! edit), finalized with the post-edit digest after a landed commit,
//! dropped after a provably clean refusal, and kept as `Uncertain` when
//! the outcome is unknown. A screened file (credential path, secret
//! content, database export) is edited without a checkpoint and the tool
//! result says so. The recorder owns no policy beyond the enable flag:
//! screening and quota are the store's gates, surfaced here as a
//! model-safe refusal or an honest "not recorded" class. The session
//! wiring builds the recorder from the session's recovery handle through
//! `DatabaseTools::with_recovery`; the one-shot `ask` path leaves the
//! tools' field `None`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use saya_store::FsSessionStore;

mod capture;
mod classify;
mod lifecycle;

pub(in crate::agent::tools) use capture::{Capture, Pending};
pub(in crate::agent::tools) use classify::{
    RefusalClass, classify_edit, classify_write, quota_refusal_text, screen_class_text,
};
// The write tool's refusal texts, re-exported for the sibling tests only:
// the classification itself consumes them through `classify`.
#[cfg(test)]
pub(in crate::agent::tools) use classify::{
    FILE_CHANGED_SINCE_OBSERVED, FILE_EXISTS_REQUIRES_DIGEST, MISSING_FILE_NOT_A_REPLACE,
};

/// Captures the before-images of one session's direct file-tool edits.
/// The recorder owns no policy beyond the enable flag: quota and
/// screening are the store's gates. A clean refusal of the edit never
/// leaves a checkpoint behind, and an ambiguous failure never reads as a
/// committed one.
#[derive(Clone)]
pub(in crate::agent::tools) struct CheckpointRecorder {
    store: Arc<FsSessionStore>,
    session_id: String,
    enabled: Arc<AtomicBool>,
}

impl CheckpointRecorder {
    /// Builds a recorder over one session's store and enable flag. The
    /// session wiring builds it from `RecoveryHandle::parts` through
    /// `DatabaseTools::with_recovery`; the flag stays the recorder's only
    /// policy.
    pub(in crate::agent::tools) fn new(
        store: Arc<FsSessionStore>,
        session_id: String,
        enabled: Arc<AtomicBool>,
    ) -> Self {
        Self {
            store,
            session_id,
            enabled,
        }
    }

    /// Whether recovery is currently on: only then do the tools capture.
    pub(super) fn active(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    /// The one clock every checkpoint timestamp comes from.
    pub(super) fn now_unix_ms(&self) -> i64 {
        crate::interactive::session_continuation::now_unix_ms()
    }
}

/// The generic capture refusal: the store could not record the edit, so
/// the edit must not run unrecorded by choice.
const CAPTURE_FAILED: &str =
    "recovery is enabled but the checkpoint could not be recorded; the file was not changed";
