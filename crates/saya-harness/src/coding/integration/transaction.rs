//! Journaled, guarded apply-back of accepted child deltas into the user's
//! original working tree. Honesty contract: this is NOT globally atomic — the
//! guard is per file; a before-image of every Modified/Deleted file is
//! captured before its first write; the durable intent stays open on any
//! non-success (no outcome is recorded unless every file is done), so
//! recovery is mandatory. Ownership is renewed before intent; every file
//! write re-proves the baseline digest. An editor racing inside a guarded
//! helper's fresh-read-to-rename window cannot be excluded by this operation.
//! Bytes and executable mode publish separately: recovery must reconcile both.

use std::fmt;
use std::num::NonZeroU64;
use std::path::Path;
use std::time::Duration;

use saya_agent::CancellationToken;
use saya_store::{SqliteStateStore, StoreError};
use saya_types::{AgentId, CheckoutOperationState, JobId};

use crate::coding::budget::{JobEventSequencer, SequencerError};
use crate::coding::git::{GitRepository, GitService};
use crate::coding::snapshot::{CapturedSnapshot, SnapshotArtifacts};
use crate::coding::worktree::{OwnershipError, VerifiedCheckout};
use crate::workspace::Workspace;

use super::apply::{AcceptedDelta, ApplyError, OverlapConflict, detect_overlaps};
use super::delta_types::{DEFAULT_DELTA_DEADLINE, DeltaKind, PathDelta};
use super::index::PlanIndex;
use super::preflight::{
    ApplyBackPreflight, PreflightBounds, PreflightError, PreflightInput, preflight_apply_back_at,
};
use super::transaction_events::{
    append_apply_back_intent, preflight_apply_back_admission, record_apply_back_outcome,
};
use super::transaction_identity::verify_apply_back_checkouts;
use super::transaction_journal::{
    ApplyBackJournal, BeforeImageStore, JournalError, JournalEvent, JournalKind,
};

/// The inputs of one apply-back run against the user's original checkout.
/// `state_dir` must be trusted and outside the original checkout, otherwise
/// journal artifacts themselves would change the user's working tree.
pub struct ApplyBackInput<'a> {
    pub store: &'a SqliteStateStore,
    pub sequencer: &'a JobEventSequencer,
    pub job_id: &'a JobId,
    pub agent_id: Option<&'a AgentId>,
    pub repository: &'a GitRepository,
    pub service: &'a GitService,
    pub candidate: &'a VerifiedCheckout,
    pub baseline: &'a CapturedSnapshot,
    pub plan: &'a PlanIndex,
    pub accepted: &'a [AcceptedDelta<'a>],
    pub artifacts: &'a SnapshotArtifacts,
    pub state_dir: &'a Path,
}

/// Bounded work for one apply-back run: a deadline for the re-run preflight
/// and an optional file limit that stops before starting the next file.
pub struct ApplyBackBounds {
    pub deadline: Duration,
    pub stop_after_files: Option<usize>,
}

impl Default for ApplyBackBounds {
    fn default() -> Self {
        Self {
            deadline: DEFAULT_DELTA_DEADLINE,
            stop_after_files: None,
        }
    }
}

/// Why an apply-back did not complete. Displays are static: no paths.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ApplyBackError {
    #[error("the state store could not be updated")]
    Store(#[from] StoreError),
    #[error("the event sequencer refused the operation")]
    Sequencer(#[from] SequencerError),
    #[error("the apply-back preflight failed")]
    Preflight(#[from] PreflightError),
    #[error("the original working tree no longer matches the captured baseline")]
    Collision(ApplyBackPreflight),
    #[error("the accepted deltas overlap")]
    Overlap(OverlapConflict),
    #[error("a delta could not be applied")]
    Apply(#[from] ApplyError),
    #[error("the apply-back journal failed")]
    Journal(#[from] JournalError),
    #[error("a checkout could not be re-verified as owned")]
    Ownership(#[from] OwnershipError),
    #[error("a verified checkout changed since it was verified")]
    StaleVerification,
    #[error("the plan, baseline and checkouts do not identify the same captured state")]
    SnapshotMismatch,
    #[error("the job is not running")]
    JobNotRunning,
    #[error("the job has reached its checkout operation cap")]
    OperationCapReached,
    #[error("the candidate checkout is not active")]
    CandidateNotActive,
    #[error("the candidate checkout already has an open operation")]
    CandidateBusy,
    #[error("the agent does not match the candidate checkout")]
    AgentMismatch,
    #[error("the apply-back was cancelled")]
    Cancelled,
    #[error("the apply-back stopped at its per-run file bound")]
    Stopped,
}

/// One completed apply-back run. Debug prints counts only.
pub struct AppliedApplyBack {
    operation_sequence: NonZeroU64,
    applied_files: usize,
}

impl AppliedApplyBack {
    pub fn operation_sequence(&self) -> NonZeroU64 {
        self.operation_sequence
    }

    pub fn applied_files(&self) -> usize {
        self.applied_files
    }
}

impl fmt::Debug for AppliedApplyBack {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppliedApplyBack")
            .field("operation_sequence", &self.operation_sequence)
            .field("applied_files", &self.applied_files)
            .finish()
    }
}

/// Applies every accepted delta into the user's original working tree, one
/// journaled, guarded file at a time. Any error after the intent is appended
/// returns the error with no recorded outcome: the intent stays open and
/// recovery is its only closer.
pub async fn apply_back(
    input: &ApplyBackInput<'_>,
    cancellation: &CancellationToken,
    bounds: ApplyBackBounds,
) -> Result<AppliedApplyBack, ApplyBackError> {
    if cancellation.is_cancelled() {
        return Err(ApplyBackError::Cancelled);
    }
    preflight_apply_back_admission(input).await?;
    if input.plan.digest() != input.baseline.plan_manifest_sha256() {
        return Err(ApplyBackError::SnapshotMismatch);
    }
    let user_ws = Workspace::open(input.repository.root()).map_err(|_| ApplyError::Fs)?;
    let preflight = preflight_apply_back_at(
        &PreflightInput {
            repository: input.repository,
            service: input.service,
            baseline: input.baseline,
            plan: input.plan,
            accepted: input.accepted,
        },
        &user_ws,
        cancellation,
        PreflightBounds {
            deadline: bounds.deadline,
        },
    )
    .await?;
    if !preflight.is_clean() {
        return Err(ApplyBackError::Collision(preflight));
    }
    detect_overlaps(input.accepted).map_err(ApplyBackError::Overlap)?;
    verify_apply_back_checkouts(input, cancellation).await?;
    let operation_sequence = append_apply_back_intent(input).await?;
    let mut journal = ApplyBackJournal::create(input.state_dir, input.job_id, operation_sequence)?;
    let mut images = BeforeImageStore::new(journal.dir());
    let mut applied_files: usize = 0;
    for accepted in input.accepted {
        let child_ws = Workspace::open(Path::new(accepted.verified().path().as_str()))
            .map_err(|_| ApplyError::Fs)?;
        for entry in accepted.delta().entries() {
            if cancellation.is_cancelled() {
                return Err(ApplyBackError::Cancelled);
            }
            if let Some(limit) = bounds.stop_after_files
                && applied_files >= limit
            {
                return Err(ApplyBackError::Stopped);
            }
            let (baseline, image) = if entry.kind() == DeltaKind::Added {
                (None, None)
            } else {
                let baseline_entry = input
                    .baseline
                    .entries()
                    .iter()
                    .find(|e| e.path().as_str() == entry.path())
                    .ok_or(ApplyError::ContentMismatch)?;
                let image =
                    images.capture(&user_ws, entry.path(), baseline_entry.content_sha256())?;
                (
                    Some(baseline_entry.content_sha256().as_str().to_owned()),
                    Some(image),
                )
            };
            journal.append(&JournalEvent::Intent {
                path: entry.path().to_owned(),
                kind: journal_kind(entry.kind()),
                baseline,
                delta: entry.content_sha256().map(|d| d.as_str().to_owned()),
                image,
            })?;
            super::apply::apply_entry(&user_ws, &child_ws, entry, input.baseline)?;
            super::apply_fs::sync_applied_entry(
                &user_ws,
                entry.path(),
                entry.kind() == DeltaKind::Deleted,
            )?;
            journal.append(&JournalEvent::Done {
                path: entry.path().to_owned(),
                result: applied_result(entry),
            })?;
            applied_files += 1;
        }
    }
    record_apply_back_outcome(input, operation_sequence, CheckoutOperationState::Completed).await?;
    Ok(AppliedApplyBack {
        operation_sequence,
        applied_files,
    })
}

fn journal_kind(kind: DeltaKind) -> JournalKind {
    match kind {
        DeltaKind::Added => JournalKind::Added,
        DeltaKind::Modified => JournalKind::Modified,
        DeltaKind::Deleted => JournalKind::Deleted,
    }
}

/// The digest the applied bytes are proven to have: the receipt digest for
/// Added/Modified and none for Deleted, where the file no longer exists.
fn applied_result(entry: &PathDelta) -> Option<String> {
    if entry.kind() == DeltaKind::Deleted {
        None
    } else {
        entry.content_sha256().map(|d| d.as_str().to_owned())
    }
}
