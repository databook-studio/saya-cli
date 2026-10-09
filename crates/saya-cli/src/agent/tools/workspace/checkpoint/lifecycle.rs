//! The checkpoint lifecycle around one file-tool edit: `begin` prepares
//! the before-image before any byte is written (a capture refusal
//! prevents the edit; a screened file proceeds unrecorded and declared),
//! `finish_label` finalizes a landed edit with its post-edit digest, and
//! `settle` drops a record after a provably clean refusal or keeps it as
//! `Uncertain` when the effect may have landed.

use saya_store::{CheckpointStoreError, NewCheckpoint};
use saya_types::{CheckpointPolicy, EditSource, FileVersion};

use super::{CAPTURE_FAILED, Capture, CheckpointRecorder, Pending, RefusalClass};
use super::{quota_refusal_text, screen_class_text};

impl CheckpointRecorder {
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
