//! Merges one receipted, accepted child delta into the root's agent-less
//! working checkout W by reusing the journaled integrate apply path: W is the
//! merge target and the ORIGINAL published baseline is the guard reference,
//! so a path W already changed refuses the child's entry fail-closed.

use std::num::NonZeroU64;
use std::time::Duration;

use saya_agent::CancellationToken;
use saya_store::SqliteStateStore;
use saya_types::{JobId, Sha256Digest};

use crate::coding::budget::JobEventSequencer;
use crate::coding::git::GitService;
use crate::coding::snapshot::CapturedSnapshot;
use crate::coding::worktree::VerifiedCheckout;

use super::apply::AcceptedDelta;
use super::integrate::{IntegrateError, IntegrateInput, integrate_candidate};

/// Borrowed handles for one merge-into-working run. `working` is the root's
/// managed checkout W (agent-less, `Active`); `baseline` is the ORIGINAL
/// published snapshot W was materialized from, not W's current state; the
/// per-entry apply guards compare W's touched paths against it, so a path W
/// already changed refuses the child's entry. `accepted` is one receipted,
/// accepted child delta to merge.
pub struct MergeInput<'a> {
    pub store: &'a SqliteStateStore,
    pub sequencer: &'a JobEventSequencer,
    pub service: &'a GitService,
    pub job_id: &'a JobId,
    /// The root's working checkout W. Ownership `agent_id` must be `None`.
    pub working: &'a VerifiedCheckout,
    /// The ORIGINAL published baseline snapshot (not W's current state).
    pub baseline: &'a CapturedSnapshot,
    /// One receipted child delta to merge into W.
    pub accepted: &'a AcceptedDelta<'a>,
}

/// Why a merge did not complete.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MergeError {
    #[error(transparent)]
    Integrate(#[from] IntegrateError),
    #[error("the working checkout is not agent-less")]
    WorkingCheckoutOwned,
}

/// The result of merging one child delta into W. `source_sha256` is W's
/// post-merge content digest — a witness, not a prepared-candidate receipt.
/// A conflict (W changed a child path) surfaces as
/// `MergeError::Integrate(IntegrateError::Apply(..))` with the outcome
/// journaled `Failed`; the caller maps that to its failure class.
pub struct MergedIntoWorking {
    operation_sequence: NonZeroU64,
    applied: usize,
    source_sha256: Sha256Digest,
}

impl MergedIntoWorking {
    pub fn operation_sequence(&self) -> NonZeroU64 {
        self.operation_sequence
    }

    pub fn applied(&self) -> usize {
        self.applied
    }

    pub fn source_sha256(&self) -> &Sha256Digest {
        &self.source_sha256
    }
}

/// Merges one receipted child delta into the root's working checkout W,
/// reusing the journaled integrate apply path: preflight admission, overlap
/// proof, intent append, serial apply with per-entry baseline guards against
/// the ORIGINAL baseline, source digest, outcome record. The merge is an
/// owner operation attributed to `agent_id` `None`, matching W's agent-less
/// ownership. Conflicts fail closed into a `Failed` outcome, never an open
/// one.
pub async fn merge_into_working(
    input: &MergeInput<'_>,
    cancellation: &CancellationToken,
    deadline: Duration,
) -> Result<MergedIntoWorking, MergeError> {
    // The merge is an owner operation attributed to `agent_id` `None`, so an
    // agent-owned checkout is the wrong target for it; the refusal happens
    // before anything is journaled and before any apply work.
    if input.working.agent_id().is_some() {
        return Err(MergeError::WorkingCheckoutOwned);
    }
    // W is the merge target and the ORIGINAL baseline is the guard reference:
    // the per-entry apply guards compare each child-touched path against it,
    // so a path W already changed refuses the child's entry (`ApplyError`),
    // and `integrate_candidate` journals the outcome `Failed`/`Uncertain`,
    // never leaving W under an open intent.
    let integrated = integrate_candidate(
        &IntegrateInput {
            store: input.store,
            sequencer: input.sequencer,
            service: input.service,
            job_id: input.job_id,
            agent_id: None,
            candidate: input.working,
            baseline: input.baseline,
            accepted: std::slice::from_ref(input.accepted),
        },
        cancellation,
        deadline,
    )
    .await?;
    Ok(MergedIntoWorking {
        operation_sequence: integrated.operation_sequence(),
        applied: integrated.applied(),
        source_sha256: integrated.source_sha256().clone(),
    })
}
