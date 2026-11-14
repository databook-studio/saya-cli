//! The checkpoint lifecycle around one file-tool edit: `begin` prepares
//! the before-image before any byte is written, `finish_label` finalizes a
//! landed edit, and `settle` drops a record after a provably clean refusal
//! or keeps it as `Uncertain` when the effect may have landed. The session
//! and job recorders implement the same three steps over different stores.

use saya_types::EditSource;

use super::{Capture, CheckpointRecorder, Pending, RefusalClass};

impl CheckpointRecorder {
    /// Prepares the edit's before-image before any byte is written. A
    /// refusal prevents the edit; a path a store will not record proceeds
    /// without a checkpoint and is declared so.
    pub(in crate::agent::tools) fn begin(
        &self,
        path: &str,
        source: EditSource,
        before: Option<&[u8]>,
    ) -> Capture {
        match self {
            Self::Session(recorder) => recorder.begin(path, source, before),
            Self::Job(recorder) => recorder.begin(path, before),
        }
    }

    /// The result label for a completed edit. A job's before-image is
    /// silent on success, so its results stay byte-identical.
    pub(in crate::agent::tools) fn finish_label(
        &self,
        capture: Option<&Capture>,
        after_sha256: &str,
        after_size: u64,
    ) -> Option<String> {
        match self {
            Self::Session(recorder) => recorder.finish_label(capture, after_sha256, after_size),
            Self::Job(recorder) => recorder.finish_label(capture, after_sha256, after_size),
        }
    }

    /// Settles a prepared record after a refused commit. Best effort.
    pub(in crate::agent::tools) fn settle(&self, pending: &Pending, class: RefusalClass) {
        match self {
            Self::Session(recorder) => recorder.settle(pending, class),
            Self::Job(recorder) => recorder.settle(pending, class),
        }
    }
}
