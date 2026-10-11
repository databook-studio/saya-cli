//! DB-only ownership verification for managed checkouts.
//!
//! [`verify_owned_checkout`] re-derives every fact from the durable R14
//! record, the live Git identity, and the published snapshot artifacts. It
//! never trusts caller input and never writes the store, Git state, or the
//! filesystem. A path alone is never authority: every consumer (child
//! command confinement, receipts, tool adapters) must call this at use time
//! and use the returned [`VerifiedCheckout`] values.
//!
//! Verification holds only at the moment of the call. It does not remove later races: the
//! checkout, its Git identity, or the artifacts can change after it returns, so consumers must
//! re-verify at use time and must not cache a [`VerifiedCheckout`] as standing authority.

use crate::coding::git::{GitError, GitRepository, GitService};
use crate::coding::snapshot::{SnapshotArtifacts, SnapshotError};
use saya_agent::CancellationToken;
use saya_store::{SqliteStateStore, StoreError};
use saya_types::{
    AgentId, GitObjectId, JobId, ManagedCheckoutId, ManagedCheckoutLifecycle, ManagedCheckoutPath,
    Sha256Digest, SnapshotId,
};

use std::fmt;

/// Borrowed handles for one [`verify_owned_checkout`] run.
pub struct OwnershipInput<'a> {
    pub store: &'a SqliteStateStore,
    pub service: &'a GitService,
    pub repository: &'a GitRepository,
    pub artifacts: &'a SnapshotArtifacts,
    pub job_id: &'a JobId,
    pub checkout_id: &'a ManagedCheckoutId,
}

/// The verified identity of an owned checkout, built from the durable
/// record — never from caller input.
pub struct VerifiedCheckout {
    checkout_id: ManagedCheckoutId,
    path: ManagedCheckoutPath,
    base_commit: GitObjectId,
    snapshot_id: SnapshotId,
    snapshot_sha256: Sha256Digest,
    plan_sha256: Sha256Digest,
    agent_id: Option<AgentId>,
}

impl VerifiedCheckout {
    pub fn checkout_id(&self) -> &ManagedCheckoutId {
        &self.checkout_id
    }

    pub fn path(&self) -> &ManagedCheckoutPath {
        &self.path
    }

    pub fn base_commit(&self) -> &GitObjectId {
        &self.base_commit
    }

    pub fn snapshot_id(&self) -> &SnapshotId {
        &self.snapshot_id
    }

    pub fn snapshot_sha256(&self) -> &Sha256Digest {
        &self.snapshot_sha256
    }

    pub fn plan_sha256(&self) -> &Sha256Digest {
        &self.plan_sha256
    }

    pub fn agent_id(&self) -> Option<&AgentId> {
        self.agent_id.as_ref()
    }
}

impl fmt::Debug for VerifiedCheckout {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedCheckout")
            .field("checkout_id", &self.checkout_id)
            .finish_non_exhaustive()
    }
}

/// Why ownership verification refused. Displays never carry paths or bytes.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum OwnershipError {
    #[error("the local state store failed")]
    Store(#[from] StoreError),
    #[error("the live git identity of the managed checkout does not verify")]
    Git(#[from] GitError),
    #[error("the published snapshot does not verify")]
    Snapshot(#[from] SnapshotError),
    #[error("no managed checkout exists under this id for this job")]
    NotFound,
    #[error("the managed checkout is not active")]
    NotActive,
    #[error("the managed checkout is owned by another job")]
    WrongOwner,
    #[error("the live repository identity differs from recorded ownership")]
    RepositoryMismatch,
    #[error("the managed checkout path does not match the ownership record")]
    PathMismatch,
    #[error("the published snapshot does not match the ownership record")]
    SnapshotMismatch,
    #[error("no plan bytes are recorded for this checkout")]
    PlanMissing,
    #[error("the recorded plan digest does not match the published snapshot")]
    PlanMismatch,
}

/// Verifies one managed checkout's ownership at use time. Read-only: it
/// never writes the store, git, or the filesystem.
pub async fn verify_owned_checkout(
    input: &OwnershipInput<'_>,
    cancellation: &CancellationToken,
) -> Result<VerifiedCheckout, OwnershipError> {
    verify_checkout_state(input, cancellation, ManagedCheckoutLifecycle::Active).await
}

pub(super) async fn verify_checkout_state(
    input: &OwnershipInput<'_>,
    cancellation: &CancellationToken,
    expected: ManagedCheckoutLifecycle,
) -> Result<VerifiedCheckout, OwnershipError> {
    if let Some(owner) = input
        .store
        .managed_checkout_owner(input.checkout_id)
        .await?
        && owner != *input.job_id
    {
        return Err(OwnershipError::WrongOwner);
    }
    let Some(record) = input
        .store
        .get_managed_checkout(input.job_id, input.checkout_id)
        .await?
    else {
        return Err(OwnershipError::NotFound);
    };
    if record.current_lifecycle != expected {
        return Err(OwnershipError::NotActive);
    }
    if record.ownership.managed_path() != &input.service.managed_path(input.checkout_id)? {
        return Err(OwnershipError::PathMismatch);
    }
    let inventory = input
        .service
        .inventory_repository(input.repository, cancellation)
        .await
        .map_err(|_| OwnershipError::RepositoryMismatch)?;
    if inventory.identity().repository_id() != record.ownership.repository_id()
        || inventory.identity().common_dir_sha256() != record.ownership.common_dir_sha256()
    {
        return Err(OwnershipError::RepositoryMismatch);
    }
    let _adopted = input
        .service
        .adopt_worktree(
            input.repository,
            record.ownership.base_commit(),
            input.checkout_id,
            cancellation,
        )
        .await?;
    let published = input
        .artifacts
        .read_snapshot(record.ownership.snapshot_id())?;
    if published.captured_manifest_sha256() != record.ownership.snapshot_sha256() {
        return Err(OwnershipError::SnapshotMismatch);
    }
    let Some(plan) = input
        .store
        .get_checkout_plan_bytes(input.job_id, input.checkout_id)
        .await?
    else {
        return Err(OwnershipError::PlanMissing);
    };
    if plan.plan_sha256 != *published.plan_manifest_sha256() {
        return Err(OwnershipError::PlanMismatch);
    }
    Ok(VerifiedCheckout {
        checkout_id: record.ownership.managed_checkout_id().clone(),
        path: record.ownership.managed_path().clone(),
        base_commit: record.ownership.base_commit().clone(),
        snapshot_id: record.ownership.snapshot_id().clone(),
        snapshot_sha256: record.ownership.snapshot_sha256().clone(),
        plan_sha256: plan.plan_sha256,
        agent_id: record.ownership.agent_id().cloned(),
    })
}
