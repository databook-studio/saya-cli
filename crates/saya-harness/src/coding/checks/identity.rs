//! Immutable run provenance; policy and epoch are recorded, not live approval.
use super::CheckExecutionError;
use crate::coding::{
    policy::ChildAuthorityView,
    worktree::{OwnershipInput, VerifiedCheckout, verify_owned_checkout},
};
use saya_agent::CancellationToken;
use saya_types::{
    AgentId, GitObjectId, ManagedCheckoutId, ManagedCheckoutPath, RepositoryId, Sha256Digest,
    SnapshotId,
};

#[derive(PartialEq, Eq)]
pub(super) struct CheckLineage {
    pub(super) checkout: ManagedCheckoutId,
    pub(super) path: ManagedCheckoutPath,
    pub(super) repository: RepositoryId,
    pub(super) snapshot: SnapshotId,
    pub(super) snapshot_sha256: Sha256Digest,
    pub(super) plan_sha256: Sha256Digest,
    pub(super) base: GitObjectId,
    pub(super) agent: Option<AgentId>,
    pub(super) policy: Sha256Digest,
    pub(super) epoch: u64,
}

impl CheckLineage {
    pub(super) fn from_run(
        candidate: &VerifiedCheckout,
        repository: &RepositoryId,
        authority: &ChildAuthorityView,
    ) -> Self {
        Self {
            checkout: candidate.checkout_id().clone(),
            path: candidate.path().clone(),
            repository: repository.clone(),
            snapshot: candidate.snapshot_id().clone(),
            snapshot_sha256: candidate.snapshot_sha256().clone(),
            plan_sha256: candidate.plan_sha256().clone(),
            base: candidate.base_commit().clone(),
            agent: candidate.agent_id().cloned(),
            policy: authority.policy_digest().clone(),
            epoch: authority.revocation_epoch(),
        }
    }

    /// Renews only facts the ownership owner can establish. Fresh approval and
    /// live revocation remain the coordinator's responsibility before effects.
    pub(super) async fn verify(
        &self,
        ownership: &OwnershipInput<'_>,
        token: &CancellationToken,
    ) -> Result<VerifiedCheckout, CheckExecutionError> {
        let current = verify_owned_checkout(ownership, token).await?;
        let snapshot = ownership.artifacts.read_snapshot(current.snapshot_id())?;
        if current.checkout_id() != &self.checkout
            || current.path() != &self.path
            || current.snapshot_id() != &self.snapshot
            || current.snapshot_sha256() != &self.snapshot_sha256
            || current.plan_sha256() != &self.plan_sha256
            || current.base_commit() != &self.base
            || current.agent_id() != self.agent.as_ref()
            || snapshot.repository_id() != &self.repository
            || snapshot.captured_manifest_sha256() != &self.snapshot_sha256
            || snapshot.plan_manifest_sha256() != &self.plan_sha256
        {
            return Err(CheckExecutionError::StaleReceipt);
        }
        Ok(current)
    }
}
