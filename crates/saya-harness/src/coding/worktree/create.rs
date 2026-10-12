//! The bounded create orchestration. The state machine and the outcome
//! classification it implements live in the [`worktree`](super) module docs.

use super::create_error::CreateCheckoutError;
use super::create_events::{
    append_create_intent, inventory_identity, preflight_create_admission, record_outcome,
    settle_git_outcome, settle_snapshot_outcome,
};
use super::create_preflight::check_checkout_identity;
use crate::coding::budget::JobEventSequencer;
use crate::coding::git::{GitRepository, GitService};
use crate::coding::snapshot::{
    SnapshotArtifacts, SnapshotManifest, materialize_snapshot, privatize_materialization_target,
    reset_materialization_target,
};
use saya_agent::CancellationToken;
use saya_store::{CheckoutPlanAppend, SqliteStateStore, StoreError};
use saya_types::{
    AgentId, CheckoutOperationState, GitObjectId, JobId, ManagedCheckoutId,
    ManagedCheckoutLifecycle,
};
use std::path::Path;

mod locks;
mod result;

use locks::create_lock;
pub use result::CreatedCheckout;

/// Borrowed handles for one [`create_managed_checkout`] run.
pub struct CreateCheckoutInput<'a> {
    pub store: &'a SqliteStateStore,
    /// Drains the ledger's one contiguous counter; the store's checkout
    /// appends route through it under the ledger mutex.
    pub sequencer: &'a JobEventSequencer,
    pub service: &'a GitService,
    /// The original repository, opened read-only; never written.
    pub repository: &'a GitRepository,
    pub artifacts: &'a SnapshotArtifacts,
    /// The PLAN manifest; its canonical bytes are persisted at bind time.
    pub plan: &'a SnapshotManifest,
    pub job_id: &'a JobId,
    /// The owning child, when one is registered for the job.
    pub agent_id: Option<AgentId>,
    pub checkout_id: &'a ManagedCheckoutId,
    pub base_commit: &'a GitObjectId,
}

/// Creates (or idempotently replays) one managed checkout.
///
/// Create calls for the same (job, checkout) id are serialized in-process
/// across the whole read–effect–outcome run; cross-process serialization is
/// not provided and the store itself enforces one durable outcome.
pub async fn create_managed_checkout(
    input: &CreateCheckoutInput<'_>,
    cancellation: &CancellationToken,
) -> Result<CreatedCheckout, CreateCheckoutError> {
    // One lock per (job, checkout) id; fetched or inserted before anything
    // is read or written and held to the end of the function. The caller
    // keeps the strong Arc while it holds/awaits the lock, so concurrent
    // same-id callers share one mutex.
    let create_lock = create_lock(input.job_id.as_str(), input.checkout_id.as_str());
    let _create_guard = create_lock.lock().await;

    // One job owns a checkout id for ever: refuse a cross-job create before any read.
    check_checkout_identity(input.store, input.job_id, input.checkout_id).await?;

    // Step 1: pre-validate the plan against the published snapshot without
    // touching the ledger.
    let published = input.artifacts.read_snapshot(input.plan.snapshot_id())?;
    if published.plan_manifest_sha256() != input.plan.manifest_sha256()
        || published.snapshot_id() != input.plan.snapshot_id()
    {
        return Err(CreateCheckoutError::PlanDigestMismatch);
    }
    let snapshot_sha256 = published.captured_manifest_sha256().clone();
    let managed_path = input.service.managed_path(input.checkout_id)?;

    // Step 2: the durable record decides replay, retry, or refusal. Caller
    // input is first validated against the recorded ownership (base commit,
    // snapshot id and captured digest, repository and common-dir identity,
    // managed path, agent); any difference is `InputMismatch`, recorded
    // nowhere. Every replay then uses the recorded values, never the
    // caller's.
    let stored = input
        .store
        .get_managed_checkout(input.job_id, input.checkout_id)
        .await?;
    // Reaching past this match means the record was absent or `Creating`:
    // an `Active` record returns above and every other lifecycle is
    // terminal.
    let retrying = stored.is_some();
    let replay_base_commit;
    let open_operation = match stored {
        Some(record) => {
            let (repository_id, common_dir_sha256) =
                inventory_identity(input, cancellation).await?;
            if record.ownership.base_commit() != input.base_commit
                || record.ownership.snapshot_id() != input.plan.snapshot_id()
                || record.ownership.snapshot_sha256() != &snapshot_sha256
                || record.ownership.repository_id() != &repository_id
                || record.ownership.common_dir_sha256() != &common_dir_sha256
                || record.ownership.managed_path() != &managed_path
                || record.ownership.agent_id() != input.agent_id.as_ref()
            {
                return Err(CreateCheckoutError::InputMismatch);
            }
            replay_base_commit = record.ownership.base_commit().clone();
            match record.current_lifecycle {
                ManagedCheckoutLifecycle::Active => {
                    let worktree = input
                        .service
                        .adopt_worktree(
                            input.repository,
                            &replay_base_commit,
                            input.checkout_id,
                            cancellation,
                        )
                        .await?;
                    return Ok(CreatedCheckout::replayed(
                        input.checkout_id.clone(),
                        worktree.path().clone(),
                    ));
                }
                ManagedCheckoutLifecycle::Creating => record.latest_operation.operation_sequence(),
                ManagedCheckoutLifecycle::Failed
                | ManagedCheckoutLifecycle::Uncertain
                | ManagedCheckoutLifecycle::Retained
                | ManagedCheckoutLifecycle::Removing
                | ManagedCheckoutLifecycle::Removed => return Err(CreateCheckoutError::Terminal),
                // `ManagedCheckoutLifecycle` is `#[non_exhaustive]`: any future
                // lifecycle label is treated as settled too — an unknown state
                // is never re-created over.
                _ => return Err(CreateCheckoutError::Terminal),
            }
        }
        None => {
            // Preflight the store's predictable intent refusals so a refused
            // append can never poison the ledger owner; the store keeps the
            // same checks for races.
            preflight_create_admission(input, cancellation).await?;
            replay_base_commit = input.base_commit.clone();
            append_create_intent(input, managed_path.clone(), snapshot_sha256, cancellation).await?
        }
    };

    // Step 3: path state. A missing path is created from the recorded base
    // commit without checking it out; any present path (including one that
    // cannot be statted) is adopted, which re-verifies git identity or
    // refuses with a typed error.
    let created = match std::fs::symlink_metadata(Path::new(managed_path.as_str())) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            input
                .service
                .add_worktree_no_checkout(
                    input.repository,
                    &replay_base_commit,
                    input.checkout_id,
                    cancellation,
                )
                .await
        }
        Ok(_) | Err(_) => {
            input
                .service
                .adopt_worktree(
                    input.repository,
                    &replay_base_commit,
                    input.checkout_id,
                    cancellation,
                )
                .await
        }
    };
    let worktree = settle_git_outcome(input, open_operation, created).await?;
    // --no-checkout leaves no index. Initialize only this still-Creating
    // checkout's private index; existing staged content is never overwritten.
    // An interrupted initialization keeps the create intent open for replay.
    input
        .service
        .initialize_materialization_index(&worktree, cancellation)
        .await?;

    // Git created the managed directory with the process umask; materialize
    // and reset require process-owned mode 0700. Privatize the target on
    // both the fresh and the `Creating`-retry path before either runs; a
    // failure here stays transient with the intent open.
    privatize_materialization_target(Path::new(managed_path.as_str()))?;

    // Step 4: materialize the baseline into the `.git`-only worktree. On a
    // `Creating` retry, first reconcile a target a crash may have left
    // populated: return it to the `.git`-only state (safe — the checkout is
    // not `Active`, so nothing has been handed out and the target is
    // discardable) before materializing again.
    let materialize_outcome = if retrying {
        match reset_materialization_target(Path::new(managed_path.as_str())) {
            Ok(()) => materialize_snapshot(
                input.artifacts,
                input.plan.snapshot_id(),
                Path::new(managed_path.as_str()),
                cancellation,
            ),
            Err(error) => Err(error),
        }
    } else {
        materialize_snapshot(
            input.artifacts,
            input.plan.snapshot_id(),
            Path::new(managed_path.as_str()),
            cancellation,
        )
    };
    let receipt = settle_snapshot_outcome(input, open_operation, materialize_outcome).await?;

    // Step 5: persist the plan bytes; a digest conflict is terminal-class.
    match input
        .store
        .record_checkout_plan_bytes(
            input.job_id,
            input.checkout_id,
            input.plan.manifest_sha256(),
            input.plan.canonical_bytes(),
        )
        .await
    {
        Ok(CheckoutPlanAppend::Recorded | CheckoutPlanAppend::AlreadyPresentSame) => {}
        Err(StoreError::Conflict) => {
            let _ = record_outcome(input, open_operation, CheckoutOperationState::Failed).await;
            return Err(CreateCheckoutError::Store(StoreError::Conflict));
        }
        Err(error) => return Err(CreateCheckoutError::Store(error)),
    }

    // Step 6: only a fully completed flow records `Completed`.
    record_outcome(input, open_operation, CheckoutOperationState::Completed).await?;
    Ok(CreatedCheckout::completed(
        input.checkout_id.clone(),
        managed_path,
        receipt,
    ))
}
