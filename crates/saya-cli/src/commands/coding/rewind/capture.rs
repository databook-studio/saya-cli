//! Taking one checkpoint of a job's working checkout at a turn boundary.
//!
//! The state recorded is the job's own delta against its baseline: every path
//! its tools changed, and the bytes each holds now. A turn that moved nothing
//! records nothing — a repeat of the previous checkpoint is skipped rather than
//! stored twice — so the checkpoint numbers count real state changes, which is
//! what the operator rewinds to.
//!
//! Capture is best effort and never authority: a turn whose state cannot be
//! recorded (over a bound, an unreadable path, a checkout that cannot be
//! resolved) leaves no checkpoint, the run continues, and a rewind to that turn
//! is refused by name rather than restoring something partial.
//!
//! Resolving the working checkout costs a git round-trip, so a caller taking a
//! checkpoint at every turn boundary resolves it once and passes it to
//! [`NativeCodingJob::capture_rewind_at`]. The run end reuses the same
//! resolution; [`NativeCodingJob::capture_rewind`] resolves one itself and is
//! kept for tests that take a single checkpoint.

use super::apply::read_paths;
use super::{CapturedPath, RewindStore, store_root};
use crate::commands::coding::{CodingError, NativeCodingJob, job_changes};
use saya_harness::coding::worktree::VerifiedCheckout;
use saya_harness::workspace::Workspace;
use std::path::Path;

impl NativeCodingJob {
    /// This job's checkpoint store, beside its own private records.
    pub(crate) fn rewind_store(&self) -> RewindStore {
        RewindStore::open(&store_root(&self.records))
    }

    /// Resolves this job's working checkout and takes one checkpoint from it.
    /// A refusal is recorded in the store and the run is not affected.
    #[cfg(all(test, unix))]
    pub(crate) async fn capture_rewind(&self) {
        match self.verified_root_checkout().await {
            Ok(verified) => self.capture_rewind_at(verified.as_ref()).await,
            Err(error) => self.record_capture_refusal(&error),
        }
    }

    /// Takes one checkpoint from an already-resolved working checkout. `None`
    /// is a job with no working checkout, which records nothing.
    pub(crate) async fn capture_rewind_at(&self, verified: Option<&VerifiedCheckout>) {
        let store = self.rewind_store();
        match self.checkpoint(verified).await {
            Ok(()) => {
                let _ = store.clear_capture_failure();
            }
            Err(error) => self.record_capture_refusal(&error),
        }
    }

    /// Takes one checkpoint when the working checkout moved since the last one.
    /// A job without a working checkout, or one whose state is unchanged,
    /// records nothing and is not an error.
    async fn checkpoint(&self, verified: Option<&VerifiedCheckout>) -> Result<(), CodingError> {
        let Some(verified) = verified else {
            return Ok(());
        };
        let ignored = self.checkout_ignored_paths(verified).await?;
        let changes = job_changes(
            &self.application.artifacts,
            &self.records,
            verified,
            &ignored,
            &self.cancellation,
        )?;
        if changes.is_empty() {
            return Ok(());
        }
        let paths: Vec<String> = changes.into_iter().map(|change| change.path).collect();
        let workspace =
            Workspace::open(Path::new(verified.path().as_str())).map_err(CodingError::owner)?;
        let captured = read_paths(&workspace, &paths)?;
        let store = self.rewind_store();
        if self.rewind_state_unchanged(&store, &captured)? {
            return Ok(());
        }
        store.capture(
            store.next_turn()?,
            &captured,
            crate::interactive::session::continuation::now_unix_ms(),
        )
    }

    /// Records that the next turn could not be checkpointed, against a closed
    /// class: the listing says a checkpoint is missing without the store ever
    /// holding an error's own text.
    pub(crate) fn record_capture_refusal(&self, error: &CodingError) {
        let store = self.rewind_store();
        if let Ok(turn) = store.next_turn() {
            let _ = store.record_capture_failure(turn, capture_failure_class(error));
        }
    }

    /// Whether `captured` is exactly the state the newest checkpoint already
    /// holds: same paths, same content. A retried turn re-enters the loop with
    /// the checkout untouched, and re-recording it would spend a checkpoint
    /// number on nothing.
    fn rewind_state_unchanged(
        &self,
        store: &RewindStore,
        captured: &[CapturedPath],
    ) -> Result<bool, CodingError> {
        let Some(last) = store.list()?.pop() else {
            return Ok(false);
        };
        if last.entries.len() != captured.len() {
            return Ok(false);
        }
        Ok(last.entries.iter().zip(captured).all(|(entry, path)| {
            entry.path == path.path
                && entry.sha256
                    == path
                        .bytes
                        .as_ref()
                        .map(|bytes| saya_types::sha256_hex(bytes))
        }))
    }
}

/// The closed class of a failed capture: a fixed vocabulary the listing
/// reports, never an error message that could carry a path or content.
pub(crate) fn capture_failure_class(error: &CodingError) -> &'static str {
    match error {
        CodingError::RewindCaptureTooLarge => "over_bound",
        _ => "unavailable",
    }
}
