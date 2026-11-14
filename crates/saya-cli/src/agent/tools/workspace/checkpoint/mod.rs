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
use std::sync::atomic::AtomicBool;

use saya_harness::coding::local::before_image::BeforeImageStore;
use saya_harness::workspace::Workspace;
use saya_store::FsSessionStore;

mod capture;
mod classify;
mod job;
mod lifecycle;
mod session;

pub(in crate::agent::tools) use capture::{Capture, Pending};
#[cfg(test)]
pub(in crate::agent::tools) use classify::screen_class_text;
pub(in crate::agent::tools) use classify::{
    RefusalClass, classify_edit, classify_write, quota_refusal_text,
};
pub(in crate::agent::tools) use job::JobRecorder;
pub(in crate::agent::tools) use session::SessionRecorder;
// The write tool's one remaining stringly refusal text, re-exported for the
// sibling tests only: the classification itself consumes it through
// `classify`.
#[cfg(test)]
pub(in crate::agent::tools) use classify::MISSING_FILE_NOT_A_REPLACE;

/// Captures the before-image of a direct file-tool edit: the session's
/// checkpoint store, or an in-place job's before-image store. The file
/// tools call the same four methods on either.
#[derive(Clone)]
pub(in crate::agent::tools) enum CheckpointRecorder {
    Session(SessionRecorder),
    Job(JobRecorder),
}

impl CheckpointRecorder {
    /// Builds a session recorder over one session's store and enable flag.
    /// The session wiring builds it from `RecoveryHandle::parts` through
    /// `DatabaseTools::with_recovery`; the flag stays its only policy.
    pub(in crate::agent::tools) fn new(
        store: Arc<FsSessionStore>,
        session_id: String,
        enabled: Arc<AtomicBool>,
    ) -> Self {
        Self::Session(SessionRecorder::new(store, session_id, enabled))
    }

    /// Builds an in-place job's recorder over the job's shared before-image
    /// store. The turn closure reads the job's own rewind clock.
    pub(in crate::agent::tools) fn job(
        store: Arc<BeforeImageStore>,
        turn: Arc<dyn Fn() -> u32 + Send + Sync>,
        workspace: Arc<Workspace>,
    ) -> Self {
        Self::Job(JobRecorder::new(store, turn, workspace))
    }

    /// Whether this recorder is currently capturing.
    pub(in crate::agent::tools) fn active(&self) -> bool {
        match self {
            Self::Session(recorder) => recorder.active(),
            Self::Job(recorder) => recorder.active(),
        }
    }
}

/// The generic capture refusal: the store could not record the edit, so
/// the edit must not run unrecorded by choice.
const CAPTURE_FAILED: &str =
    "recovery is enabled but the checkpoint could not be recorded; the file was not changed";
