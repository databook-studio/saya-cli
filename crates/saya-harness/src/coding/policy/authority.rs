use saya_types::{
    AgentId, Capabilities, InvocationId, JobId, ManagedCheckoutId, ProfileIdentity, RepositoryId,
    Sha256Digest, SnapshotId,
};
use thiserror::Error;

use super::values;

pub(super) const MAX_DATABASE_PROFILES: usize = 32;

/// Trusted role preset selected by the coordinator, never a permission grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildRole {
    Analyst,
    Implementer,
    Reviewer,
}

/// Trusted host support that can only remove authority from a child view.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlatformSupport {
    pub workspace_edits: bool,
    pub network_fetch: bool,
    pub database_reads: bool,
}

impl PlatformSupport {
    /// The support the in-process workspace tools establish on their own.
    ///
    /// Those tools spawn no command: every write is admitted by the exact-write
    /// decider against the verified checkout and enforced again by the
    /// executor's own admission. A spawned command's confinement is proven by
    /// the sandbox where the shell that spawns it is composed, and that shell
    /// is absent where the host cannot prove one — so granting the workspace
    /// tools needs no sandbox probe on the start path.
    pub const fn workspace_tools() -> Self {
        Self {
            workspace_edits: true,
            network_fetch: false,
            database_reads: false,
        }
    }
}

/// Constructor input issued by the trusted CLI or coordinator.
#[derive(Debug)]
pub struct RootDelegationInput {
    pub job_id: JobId,
    pub agent_id: AgentId,
    pub invocation_id: InvocationId,
    pub repository_id: RepositoryId,
    pub snapshot_id: SnapshotId,
    pub managed_checkout_id: Option<ManagedCheckoutId>,
    pub policy_digest: Sha256Digest,
    pub revocation_epoch: u64,
    pub capabilities: Capabilities,
    pub database_profiles: Vec<ProfileIdentity>,
}

/// Immutable root scope that a trusted coordinator narrows for one child.
///
/// The scope carries the shared working checkout and a write capability,
/// never a path set.
#[derive(Debug)]
pub struct RootDelegationScope {
    pub(super) job_id: JobId,
    pub(super) agent_id: AgentId,
    pub(super) invocation_id: InvocationId,
    pub(super) repository_id: RepositoryId,
    pub(super) snapshot_id: SnapshotId,
    pub(super) managed_checkout_id: Option<ManagedCheckoutId>,
    pub(super) policy_digest: Sha256Digest,
    pub(super) revocation_epoch: u64,
    pub(super) capabilities: Capabilities,
    pub(super) database_profiles: Vec<ProfileIdentity>,
}

/// Model-requested attenuation inputs. Construction validates only shape and bounds.
///
/// The write grant is the `workspace_write` capability, never a path set:
/// there is no job-level ceiling, so a request carries no paths at all.
#[derive(Debug)]
pub struct ChildAuthorityRequest {
    pub(super) role: ChildRole,
    pub(super) capabilities: Capabilities,
    pub(super) database_profiles: Vec<ProfileIdentity>,
}

/// The immutable, already-narrowed policy a future child factory receives.
///
/// Cloning copies an already-narrowed immutable view; it can never widen authority (the fields are private to the policy module).
///
/// The view carries the shared working checkout's identity
/// ([`ChildAuthorityView::managed_checkout_id`]) and a write capability
/// (`capabilities.workspace_write`): a write-capable view may write any path in
/// that checkout, `.git` included; a view without the capability is
/// read-only.
#[derive(Debug, Clone)]
pub struct ChildAuthorityView {
    pub(super) job_id: JobId,
    pub(super) agent_id: AgentId,
    pub(super) invocation_id: InvocationId,
    pub(super) repository_id: RepositoryId,
    pub(super) snapshot_id: SnapshotId,
    pub(super) managed_checkout_id: Option<ManagedCheckoutId>,
    pub(super) policy_digest: Sha256Digest,
    pub(super) revocation_epoch: u64,
    pub(super) capabilities: Capabilities,
    pub(super) database_profiles: Vec<ProfileIdentity>,
}

impl RootDelegationScope {
    /// The identity this scope was issued for: the root's own trusted agent
    /// name, or the helper's. Callers use it to name per-agent state (a
    /// helper's private scratch tree) without re-deriving it.
    pub fn agent_id(&self) -> &AgentId {
        &self.agent_id
    }

    pub(super) fn from_input(input: RootDelegationInput) -> Result<Self, AuthorityError> {
        Ok(Self {
            job_id: input.job_id,
            agent_id: input.agent_id,
            invocation_id: input.invocation_id,
            repository_id: input.repository_id,
            snapshot_id: input.snapshot_id,
            managed_checkout_id: input.managed_checkout_id,
            policy_digest: input.policy_digest,
            revocation_epoch: input.revocation_epoch,
            capabilities: values::validate_capabilities(&input.capabilities)?,
            database_profiles: values::validate_profiles(input.database_profiles)?,
        })
    }

    pub fn narrow(
        &self,
        request: ChildAuthorityRequest,
        platform: PlatformSupport,
    ) -> Result<ChildAuthorityView, AuthorityError> {
        values::narrow(self, request, platform)
    }
}

impl ChildAuthorityRequest {
    /// Builds one attenuation request. The write grant is the
    /// `workspace_write` capability the caller declares; the role decides
    /// whether a capability grant can become a write at all.
    pub fn new(
        role: ChildRole,
        capabilities: Capabilities,
        database_profiles: Vec<ProfileIdentity>,
    ) -> Result<Self, AuthorityError> {
        Ok(Self {
            role,
            capabilities: values::validate_capabilities(&capabilities)?,
            database_profiles: values::validate_profiles(database_profiles)?,
        })
    }
}

/// Construction or shape failure before an authority view exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AuthorityError {
    #[error("fresh host approval for this exact root scope was denied")]
    ApprovalDenied,
    #[error("authority database profile set exceeds its item bound")]
    TooManyDatabaseProfiles,
    #[error("authority database profile set contains a duplicate")]
    DuplicateDatabaseProfile,
    #[error("authority capability scope has an invalid shape")]
    InvalidCapabilityShape,
}

/// Why the immutable child authority view refuses one exact operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AuthorityRefusal {
    #[error("child authority has no managed checkout")]
    ManagedCheckoutUnavailable,
    #[error("managed checkout does not match child authority")]
    ManagedCheckoutMismatch,
    #[error("workspace write is outside child authority")]
    WorkspaceWriteDenied,
    #[error("network destination is outside child authority")]
    NetworkDestinationDenied,
    #[error("database profile is outside child authority")]
    DatabaseProfileDenied,
}
