//! The job-scoped recorder: an in-place job's first-touch before-images.
//! It shares the four method names with [`super::SessionRecorder`], so the
//! file tools never learn which store is behind them.

use std::sync::Arc;

use saya_harness::coding::local::before_image::{BeforeImageStore, FirstTouch};
use saya_harness::workspace::Workspace;
use saya_types::WorkspacePath;

use super::CAPTURE_FAILED;
use super::capture::{Capture, Pending};
use super::classify::RefusalClass;

/// Records the first bytes each direct file-tool edit saw in one in-place
/// job. The store owns the bounds: a bound is a declared "not recorded",
/// never a refused edit. Only a store I/O failure refuses the edit.
#[derive(Clone)]
pub(in crate::agent::tools) struct JobRecorder {
    store: Arc<BeforeImageStore>,
    turn: Arc<dyn Fn() -> u32 + Send + Sync>,
    workspace: Arc<Workspace>,
}

impl JobRecorder {
    pub(in crate::agent::tools) fn new(
        store: Arc<BeforeImageStore>,
        turn: Arc<dyn Fn() -> u32 + Send + Sync>,
        workspace: Arc<Workspace>,
    ) -> Self {
        Self {
            store,
            turn,
            workspace,
        }
    }

    /// A job recorder is always capturing; the store's bounds are the only
    /// reason a path goes unrecorded.
    pub(in crate::agent::tools) fn active(&self) -> bool {
        true
    }

    /// Records the first touch before any byte is written. A bound is a
    /// declared "not recorded" and the edit proceeds; a store failure
    /// refuses the edit, matching the session recorder's policy.
    pub(in crate::agent::tools) fn begin(&self, path: &str, before: Option<&[u8]>) -> Capture {
        let Ok(path) = WorkspacePath::try_from(path.to_owned()) else {
            return Capture::Unavailable("unrecordable path");
        };
        let executable = self.workspace.is_executable(path.as_str()).unwrap_or(false);
        match self
            .store
            .record_first_touch(&path, before, executable, (self.turn)())
        {
            Ok(FirstTouch::Recorded) => Capture::Recorded(Pending {
                id: path.as_str().to_owned(),
            }),
            Ok(FirstTouch::AlreadyRecorded) => Capture::Off,
            Ok(FirstTouch::Unrecorded(reason)) => Capture::Unavailable(reason.text()),
            Err(_) => Capture::Refused(CAPTURE_FAILED.to_owned()),
        }
    }

    /// A job's successful result carries no `recovery` key: the edit-time
    /// before-image is a recovery fact, not a per-call report.
    pub(in crate::agent::tools) fn finish_label(
        &self,
        _capture: Option<&Capture>,
        _after_sha256: &str,
        _after_size: u64,
    ) -> Option<String> {
        None
    }

    /// A provably clean refusal drops the first touch; an ambiguous one
    /// keeps it, because the bytes may already have changed.
    pub(in crate::agent::tools) fn settle(&self, pending: &Pending, class: RefusalClass) {
        if class == RefusalClass::Clean {
            let _ = self.store.forget(&pending.id);
        }
    }
}
