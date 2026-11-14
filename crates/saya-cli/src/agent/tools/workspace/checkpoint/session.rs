//! The session-scoped recorder: today's `FsSessionStore` checkpoint capture
//! around direct file-tool edits, unchanged. It is one variant of
//! [`super::CheckpointRecorder`] so a job's before-image recorder can share
//! the same four method names.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use saya_store::{CheckpointStoreError, FsSessionStore, NewCheckpoint};
use saya_types::{CheckpointPolicy, EditSource, FileVersion};

use super::CAPTURE_FAILED;
use super::capture::{Capture, Pending};
use super::classify::{RefusalClass, quota_refusal_text, screen_class_text};

/// Captures the before-images of one session's direct file-tool edits.
/// The recorder owns no policy beyond the enable flag: quota and
/// screening are the store's gates. A clean refusal of the edit never
/// leaves a checkpoint behind, and an ambiguous failure never reads as a
/// committed one.
#[derive(Clone)]
pub(in crate::agent::tools) struct SessionRecorder {
    store: Arc<FsSessionStore>,
    session_id: String,
    enabled: Arc<AtomicBool>,
}

impl SessionRecorder {
    /// Builds a recorder over one session's store and enable flag.
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
    pub(in crate::agent::tools) fn active(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    /// The one clock every checkpoint timestamp comes from.
    pub(super) fn now_unix_ms(&self) -> i64 {
        crate::interactive::session::continuation::now_unix_ms()
    }

    /// Prepares the edit's before-image before any byte is written. A
    /// refusal prevents the edit; a screened file proceeds without a
    /// checkpoint and is declared so.
    pub(in crate::agent::tools) fn begin(
        &self,
        path: &str,
        source: EditSource,
        before: Option<&[u8]>,
    ) -> Capture {
        if !self.active() {
            return Capture::Off;
        }
        let policy = CheckpointPolicy { enabled: true };
        let request = NewCheckpoint {
            path,
            source,
            before,
        };
        match self
            .store
            .prepare_checkpoint(&self.session_id, &policy, request, self.now_unix_ms())
        {
            Ok(record) => Capture::Recorded(Pending {
                id: record.id.as_str().to_owned(),
            }),
            Err(CheckpointStoreError::Screened(class)) => {
                Capture::Unavailable(screen_class_text(class))
            }
            Err(CheckpointStoreError::Refused(quota)) => {
                Capture::Refused(quota_refusal_text(quota))
            }
            Err(_) => Capture::Refused(CAPTURE_FAILED.to_owned()),
        }
    }

    /// The result label for a completed edit: a landed commit is finalized
    /// with the post-edit version ("checkpoint recorded"; a failed commit
    /// stays `Prepared`, which recovery marks `Uncertain` later — the label
    /// then reads "checkpoint pending", the edit itself already succeeded);
    /// a screened edit reads "not recorded: <class>". `None` when nothing
    /// was captured.
    pub(in crate::agent::tools) fn finish_label(
        &self,
        capture: Option<&Capture>,
        after_sha256: &str,
        after_size: u64,
    ) -> Option<String> {
        let pending = match capture {
            Some(Capture::Recorded(pending)) => pending,
            Some(Capture::Unavailable(class)) => return Some(Self::label_unavailable(class)),
            _ => return None,
        };
        let after = FileVersion {
            sha256: after_sha256.to_owned(),
            size: after_size,
        };
        let label = match self.store.commit_checkpoint(
            &self.session_id,
            &pending.id,
            after,
            self.now_unix_ms(),
        ) {
            Ok(_) => "checkpoint recorded",
            Err(_) => "checkpoint pending",
        };
        Some(label.to_owned())
    }

    /// Settles a prepared record after a refused commit: a provably clean
    /// refusal (nothing was written) drops the record; an ambiguous one
    /// keeps it as `Uncertain`. Best effort either way.
    pub(in crate::agent::tools) fn settle(&self, pending: &Pending, class: RefusalClass) {
        match class {
            RefusalClass::Clean => {
                let _ = self.store.delete_checkpoint(&self.session_id, &pending.id);
            }
            RefusalClass::Ambiguous => {
                let _ = self.store.mark_checkpoint_uncertain(
                    &self.session_id,
                    &pending.id,
                    self.now_unix_ms(),
                );
            }
        }
    }

    /// The honest result label for a screened edit: edited, unrecorded.
    pub(in crate::agent::tools) fn label_unavailable(class: &str) -> String {
        format!("not recorded: {class}")
    }
}
