//! DB-only ownership verification for managed checkouts.
//!
//! [`verify_owned_checkout`] re-derives every fact from the durable
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
use crate::coding::local::{LocalRoot, LocalRootError};
use crate::coding::snapshot::{SnapshotArtifacts, SnapshotError};
use saya_agent::CancellationToken;
use saya_store::{SqliteStateStore, StoreError};
use saya_types::{
    AgentId, GitObjectId, JobId, ManagedCheckoutId, ManagedCheckoutLifecycle, ManagedCheckoutPath,
    RepositoryId, Sha256Digest, SnapshotId,
};

use std::fmt;
use std::path::Path;

/// Borrowed handles for one [`verify_owned_checkout`] run.
pub struct OwnershipInput<'a> {
    pub store: &'a SqliteStateStore,
    pub service: &'a GitService,
    pub repository: &'a GitRepository,
    pub artifacts: &'a SnapshotArtifacts,
    pub job_id: &'a JobId,
    pub checkout_id: &'a ManagedCheckoutId,
}

/// The fixed point a verified checkout was created from: the Git commit a
/// managed checkout was adopted at, or the baseline plan digest an in-place
/// folder was captured from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckoutBase {
    Git(GitObjectId),
    InPlace(Sha256Digest),
}

impl CheckoutBase {
    pub fn git(&self) -> Option<&GitObjectId> {
        match self {
            Self::Git(commit) => Some(commit),
            Self::InPlace(_) => None,
        }
    }

    pub fn in_place(&self) -> Option<&Sha256Digest> {
        match self {
            Self::Git(_) => None,
            Self::InPlace(digest) => Some(digest),
        }
    }

    pub fn is_in_place(&self) -> bool {
        matches!(self, Self::InPlace(_))
    }
}

/// The verified identity of an owned checkout, built from the durable
/// record — never from caller input.
pub struct VerifiedCheckout {
    checkout_id: ManagedCheckoutId,
    path: ManagedCheckoutPath,
    base: CheckoutBase,
    snapshot_id: SnapshotId,
    snapshot_sha256: Sha256Digest,
    plan_sha256: Sha256Digest,
    agent_id: Option<AgentId>,
}

impl VerifiedCheckout {
    /// The proof a completed create flow hands to its caller. Every field is
    /// read from the durable record this flow just wrote — never from caller
    /// input — and the record's lifecycle must be `Active`, which the
    /// `Completed` outcome it just appended established. The create flow
    /// verified the live Git identity and materialized the published snapshot
    /// moments before; the caller spends this proof within the same start
    /// flow, and anything that changes W invalidates the readiness seed that
    /// carries it.
    pub(super) fn from_completed_create(
        record: &saya_store::ManagedCheckoutRecord,
        plan_sha256: Sha256Digest,
    ) -> Result<Self, OwnershipError> {
        if record.current_lifecycle != ManagedCheckoutLifecycle::Active {
            return Err(OwnershipError::NotActive);
        }
        let ownership = &record.ownership;
        Ok(Self {
            checkout_id: ownership.managed_checkout_id().clone(),
            path: ownership.managed_path().clone(),
            base: CheckoutBase::Git(ownership.base_commit().clone()),
            snapshot_id: ownership.snapshot_id().clone(),
            snapshot_sha256: ownership.snapshot_sha256().clone(),
            plan_sha256,
            agent_id: ownership.agent_id().cloned(),
        })
    }

    pub fn checkout_id(&self) -> &ManagedCheckoutId {
        &self.checkout_id
    }

    pub fn path(&self) -> &ManagedCheckoutPath {
        &self.path
    }

    pub fn base(&self) -> &CheckoutBase {
        &self.base
    }

    /// The Git commit a managed checkout was adopted at, or `None` for an
    /// in-place root, which has no Git history.
    pub fn base_commit(&self) -> Option<&GitObjectId> {
        self.base.git()
    }

    /// The baseline plan digest an in-place root was captured from, or `None`
    /// for a managed checkout.
    pub fn in_place_baseline(&self) -> Option<&Sha256Digest> {
        self.base.in_place()
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
    #[error(transparent)]
    InPlaceRoot(#[from] LocalRootError),
    #[error("the in-place root id does not match the baseline or the durable job")]
    RootMismatch,
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

/// Verifies ownership for a checkout admitted to apply-back work. The durable
/// lifecycle must be [`ManagedCheckoutLifecycle::Active`] or
/// [`ManagedCheckoutLifecycle::Retained`] — an open or settled apply-back or
/// candidate integration retains the checkout — and the record's current
/// lifecycle is the verification expectation, never a caller claim. Read-only
/// like [`verify_owned_checkout`].
pub async fn verify_admitted_checkout(
    input: &OwnershipInput<'_>,
    cancellation: &CancellationToken,
) -> Result<VerifiedCheckout, OwnershipError> {
    let record = input
        .store
        .get_managed_checkout(input.job_id, input.checkout_id)
        .await?
        .ok_or(OwnershipError::NotFound)?;
    match record.current_lifecycle {
        ManagedCheckoutLifecycle::Active | ManagedCheckoutLifecycle::Retained => {}
        _ => return Err(OwnershipError::NotActive),
    }
    verify_checkout_state(input, cancellation, record.current_lifecycle).await
}

/// Borrowed handles for one [`verify_in_place_root`] run. `root_id`,
/// `root_path`, `snapshot_id` and `plan_sha256` are the values the durable
/// job and its private in-place record fixed at start; every one is
/// re-derived, never trusted as authority.
pub struct InPlaceRootInput<'a> {
    pub artifacts: &'a SnapshotArtifacts,
    pub state_root: &'a Path,
    pub root_id: &'a RepositoryId,
    pub root_path: &'a Path,
    pub snapshot_id: &'a SnapshotId,
    pub plan_sha256: &'a Sha256Digest,
}

/// Verifies one in-place root at use time: the folder still canonicalizes to
/// the recorded id, and the published baseline still names that id and the
/// recorded plan digest. Read-only. The returned checkout has no managed
/// checkout record and no Git base; its id is derived from the root id alone
/// so the root and every helper share it.
pub fn verify_in_place_root(
    input: &InPlaceRootInput<'_>,
) -> Result<VerifiedCheckout, OwnershipError> {
    let root = LocalRoot::open(input.root_path, input.state_root)?;
    if root.id().as_repository_id() != input.root_id {
        return Err(OwnershipError::RootMismatch);
    }
    let snapshot = input.artifacts.read_snapshot(input.snapshot_id)?;
    if snapshot.repository_id() != input.root_id {
        return Err(OwnershipError::RootMismatch);
    }
    if snapshot.plan_manifest_sha256() != input.plan_sha256 {
        return Err(OwnershipError::SnapshotMismatch);
    }
    let path = root
        .path()
        .to_str()
        .ok_or(OwnershipError::InPlaceRoot(LocalRootError::UnsupportedPath))?;
    let path = ManagedCheckoutPath::new(path)
        .map_err(|_| OwnershipError::InPlaceRoot(LocalRootError::UnsupportedPath))?;
    Ok(VerifiedCheckout {
        checkout_id: in_place_checkout_id(input.root_id)?,
        path,
        base: CheckoutBase::InPlace(input.plan_sha256.clone()),
        snapshot_id: input.snapshot_id.clone(),
        snapshot_sha256: snapshot.captured_manifest_sha256().clone(),
        plan_sha256: snapshot.plan_manifest_sha256().clone(),
        agent_id: None,
    })
}

/// The one synthetic checkout id an in-place root carries: derived from the
/// root id alone, so the root and every helper name the same checkout. It is
/// not a durable managed-checkout id and never resolves to a checkout row.
fn in_place_checkout_id(root_id: &RepositoryId) -> Result<ManagedCheckoutId, OwnershipError> {
    root_id
        .in_place_checkout_id()
        .ok_or(OwnershipError::RootMismatch)
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
    let (repository_id, common_dir_sha256) = input
        .service
        .verify_repository_identity(input.repository, cancellation)
        .await
        .map_err(|_| OwnershipError::RepositoryMismatch)?;
    if repository_id != *record.ownership.repository_id()
        || common_dir_sha256 != *record.ownership.common_dir_sha256()
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
        base: CheckoutBase::Git(record.ownership.base_commit().clone()),
        snapshot_id: record.ownership.snapshot_id().clone(),
        snapshot_sha256: record.ownership.snapshot_sha256().clone(),
        plan_sha256: plan.plan_sha256,
        agent_id: record.ownership.agent_id().cloned(),
    })
}
