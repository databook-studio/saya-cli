//! Durable checkout-event plumbing for the create flow: the admission
//! preflight, the sequencer closures and their probe-validated event
//! builders, the live inventory identity, and the outcome classifiers.
//!
//! Event construction is relationship-only validation, so each builder is
//! proven constructible with a placeholder sequence before the sequencer
//! allocates. The closure rebuilds the event at the allocated sequence and,
//! on the impossible construction failure, returns the pre-validated probe
//! event instead; the sequencer then refuses it as `SequenceMismatch`
//! before any store call — the caller-bug class that leaves the ledger
//! owner usable.

use super::create::CreateCheckoutInput;
use super::create_error::CreateCheckoutError;
use crate::coding::budget::SequencerError;
use crate::coding::git::{GitError, GitInventoryError, ManagedWorktree};
use crate::coding::snapshot::{MaterializedSnapshot, SnapshotError};
use saya_agent::CancellationToken;
use saya_store::{MAX_CHECKOUT_OPERATIONS_PER_JOB, SqliteStateStore};
use saya_types::{
    CheckoutOperationState, CheckoutOperationV1, CodingEventKind, CodingEventV1, JobId,
    ManagedCheckoutId, ManagedCheckoutLifecycle, ManagedCheckoutPath, ManagedCheckoutRecordData,
    ManagedCheckoutRecordV1, RepositoryId, RepositoryMutationAction, Sha256Digest,
};
use std::num::NonZeroU64;

/// The store's per-job checkout cap (`MAX_CHECKOUT_RECORDS_PER_JOB`).
const CHECKOUTS_PER_JOB_CAP: usize = 256;
/// Page size for the cap preflight; the store's coding page cap is 100.
const CHECKOUT_CAP_PAGE: usize = 100;

/// Preflights the store's predictable intent refusals for a checkout id
/// with no durable record: the job must be under its per-job checkout and
/// operation caps, running, bound to the repository and snapshot this
/// create writes, and (when an agent is supplied) registered for the job.
/// The store keeps the same checks for races; refusing here means a refused
/// append can never poison the ledger owner.
pub(super) async fn preflight_create_admission(
    input: &CreateCheckoutInput<'_>,
    cancellation: &CancellationToken,
) -> Result<(), CreateCheckoutError> {
    if checkouts_for_job(input.store, input.job_id).await? >= CHECKOUTS_PER_JOB_CAP {
        return Err(CreateCheckoutError::CheckoutCapReached);
    }
    let operations = input.store.count_checkout_operations(input.job_id).await?;
    if operations >= MAX_CHECKOUT_OPERATIONS_PER_JOB {
        return Err(CreateCheckoutError::OperationCapReached);
    }
    super::create_preflight::check_job_binding(input, cancellation).await?;
    super::create_preflight::check_agent_registration(input).await
}

/// Counts the job's checkouts for the cap preflight, paging with the
/// store's bounded keyset cursor until the cap is provably reached or the
/// list ends.
async fn checkouts_for_job(
    store: &SqliteStateStore,
    job_id: &JobId,
) -> Result<usize, CreateCheckoutError> {
    let mut total = 0usize;
    let mut cursor: Option<ManagedCheckoutId> = None;
    loop {
        let page = store
            .list_managed_checkouts(job_id, cursor.as_ref(), CHECKOUT_CAP_PAGE)
            .await?;
        total += page.items.len();
        if page.next_cursor.is_none() || total >= CHECKOUTS_PER_JOB_CAP {
            return Ok(total);
        }
        cursor = page.next_cursor;
    }
}

/// Appends the durable `CheckoutCreateIntent` for a checkout id with no
/// record, returning the operation sequence the store accepted.
///
/// The record data is validated with the placeholder sequence
/// [`NonZeroU64::MIN`] before the sequencer allocates; the closure rebuilds
/// the event at the allocated sequence and falls back to the probe on an
/// impossible construction failure. DTO construction can only fail on
/// inputs that can never validate; that surfaces as
/// [`SequencerError::SequenceMismatch`], the caller-bug class.
pub(super) async fn append_create_intent(
    input: &CreateCheckoutInput<'_>,
    managed_path: ManagedCheckoutPath,
    snapshot_sha256: Sha256Digest,
    cancellation: &CancellationToken,
) -> Result<NonZeroU64, CreateCheckoutError> {
    let (repository_id, common_dir_sha256) = inventory_identity(input, cancellation).await?;
    let data = ManagedCheckoutRecordData {
        job_id: input.job_id.clone(),
        agent_id: input.agent_id.clone(),
        managed_checkout_id: input.checkout_id.clone(),
        repository_id,
        common_dir_sha256,
        managed_path,
        base_commit: input.base_commit.clone(),
        snapshot_id: input.plan.snapshot_id().clone(),
        snapshot_sha256,
        created_by_sequence: NonZeroU64::MIN,
        lifecycle: ManagedCheckoutLifecycle::Creating,
    };
    let probe = intent_event(input, data.clone(), NonZeroU64::MIN)?;
    let appended = input
        .sequencer
        .append_checkout_intent(|sequence| {
            match NonZeroU64::new(sequence)
                .and_then(|sequence| intent_event(input, data.clone(), sequence).ok())
            {
                Some(event) => event,
                None => probe.clone(),
            }
        })
        .await?;
    Ok(appended.sequence)
}

/// Computes the live inventory identity for the create flow: it seeds the
/// intent record data and validates a retry's caller input against the
/// recorded ownership. Identity that cannot be re-established is the
/// `RepositoryChanged` contract: the open repository no longer proves the
/// identity it was opened with.
pub(super) async fn inventory_identity(
    input: &CreateCheckoutInput<'_>,
    cancellation: &CancellationToken,
) -> Result<(RepositoryId, Sha256Digest), CreateCheckoutError> {
    let inventory = match input
        .service
        .inventory_repository(input.repository, cancellation)
        .await
    {
        Ok(inventory) => inventory,
        Err(GitInventoryError::Git(error)) => return Err(CreateCheckoutError::Git(error)),
        Err(_) => return Err(CreateCheckoutError::Git(GitError::RepositoryChanged)),
    };
    let identity = inventory.identity();
    Ok((
        identity.repository_id().clone(),
        identity.common_dir_sha256().clone(),
    ))
}

/// Records the create outcome (`Completed`/`Failed`/`Uncertain`) against
/// the open operation. On the success path the caller propagates errors;
/// for terminal classification the caller discards the result so a
/// journaling failure never masks the primary error.
pub(super) async fn record_outcome(
    input: &CreateCheckoutInput<'_>,
    operation_sequence: NonZeroU64,
    state: CheckoutOperationState,
) -> Result<(), CreateCheckoutError> {
    // The probe validates at `operation + 1`; the placeholder value is never
    // zero, so the outer fallback below is unreachable. An operation
    // sequence at `u64::MAX` has no allocatable successor, so the probe —
    // and any real build — can only fail into the `SequenceMismatch`
    // caller-bug class.
    let probe_sequence =
        NonZeroU64::new(operation_sequence.get().checked_add(1).unwrap_or(u64::MAX))
            .unwrap_or(NonZeroU64::MIN);
    let probe = outcome_event(input, operation_sequence, state, probe_sequence)?;
    input
        .sequencer
        .append_checkout_outcome(|sequence| {
            match NonZeroU64::new(sequence)
                .and_then(|sequence| outcome_event(input, operation_sequence, state, sequence).ok())
            {
                Some(event) => event,
                None => probe.clone(),
            }
        })
        .await
        .map(|_| ())?;
    Ok(())
}

/// Builds the `CheckoutCreateIntent` event at `sequence`: the intent links
/// `sequence == operation.operation_sequence() == checkout.created_by_sequence`.
fn intent_event(
    input: &CreateCheckoutInput<'_>,
    mut data: ManagedCheckoutRecordData,
    sequence: NonZeroU64,
) -> Result<CodingEventV1, CreateCheckoutError> {
    data.created_by_sequence = sequence;
    let checkout = ManagedCheckoutRecordV1::new(data).map_err(|_| unconstructible_event())?;
    let operation = CheckoutOperationV1::new(
        input.job_id.clone(),
        sequence,
        input.checkout_id.clone(),
        RepositoryMutationAction::CreateCheckout,
        CheckoutOperationState::Intent,
    )
    .map_err(|_| unconstructible_event())?;
    CodingEventV1::new(
        sequence,
        input.job_id.clone(),
        input.agent_id.clone(),
        None,
        CodingEventKind::CheckoutCreateIntent {
            checkout: Box::new(checkout),
            operation: Box::new(operation),
        },
    )
    .map_err(|_| unconstructible_event())
}

/// Builds the `CheckoutOperationOutcome` event at `event_sequence` against
/// the open create operation (`operation_sequence < event_sequence`).
fn outcome_event(
    input: &CreateCheckoutInput<'_>,
    operation_sequence: NonZeroU64,
    state: CheckoutOperationState,
    event_sequence: NonZeroU64,
) -> Result<CodingEventV1, CreateCheckoutError> {
    let operation = CheckoutOperationV1::new(
        input.job_id.clone(),
        operation_sequence,
        input.checkout_id.clone(),
        RepositoryMutationAction::CreateCheckout,
        state,
    )
    .map_err(|_| unconstructible_event())?;
    CodingEventV1::new(
        event_sequence,
        input.job_id.clone(),
        input.agent_id.clone(),
        None,
        CodingEventKind::CheckoutOperationOutcome {
            operation: Box::new(operation),
        },
    )
    .map_err(|_| unconstructible_event())
}

/// DTO construction can only fail on inputs that can never validate (every
/// field is a pre-validated type and the payload is far below the wire
/// limits); surface it as the sequencer's caller-bug class.
fn unconstructible_event() -> CreateCheckoutError {
    CreateCheckoutError::Sequencer(SequencerError::SequenceMismatch)
}

/// Classifies one worktree-mutation outcome against the open operation:
/// an unknown outcome records `Uncertain`, base/path refusals record
/// `Failed`, and everything else stays transient with the intent open.
pub(super) async fn settle_git_outcome(
    input: &CreateCheckoutInput<'_>,
    operation_sequence: NonZeroU64,
    outcome: Result<ManagedWorktree, GitError>,
) -> Result<ManagedWorktree, CreateCheckoutError> {
    match outcome {
        Ok(worktree) => Ok(worktree),
        Err(error @ GitError::MutationOutcomeUnknown { .. }) => {
            let _ =
                record_outcome(input, operation_sequence, CheckoutOperationState::Uncertain).await;
            Err(CreateCheckoutError::Git(error))
        }
        Err(error @ (GitError::InvalidBaseCommit | GitError::ManagedPathOutsideState)) => {
            let _ = record_outcome(input, operation_sequence, CheckoutOperationState::Failed).await;
            Err(CreateCheckoutError::Git(error))
        }
        Err(error) => Err(CreateCheckoutError::Git(error)),
    }
}

/// Classifies one materialization outcome: a cleanup failure leaves the
/// target discardable and records `Failed`; everything else stays transient
/// with the intent open (a failed materialize rolls the target back to
/// `.git`-only).
pub(super) async fn settle_snapshot_outcome(
    input: &CreateCheckoutInput<'_>,
    operation_sequence: NonZeroU64,
    outcome: Result<MaterializedSnapshot, SnapshotError>,
) -> Result<MaterializedSnapshot, CreateCheckoutError> {
    match outcome {
        Ok(receipt) => Ok(receipt),
        Err(error @ SnapshotError::MaterializeCleanupFailed) => {
            let _ = record_outcome(input, operation_sequence, CheckoutOperationState::Failed).await;
            Err(CreateCheckoutError::Snapshot(error))
        }
        Err(error) => Err(CreateCheckoutError::Snapshot(error)),
    }
}
