use saya_types::{
    AgentId, Capabilities, InvocationId, JobId, ManagedCheckoutId, ProfileIdentity, RepositoryId,
    Sha256Digest, SnapshotId, WorkspacePath,
};
use thiserror::Error;

use super::values;

pub(super) const MAX_WRITE_PATHS: usize = 128;
pub(super) const MAX_WRITE_PATH_BYTES: usize = 512;
pub(super) const MAX_WRITE_PATH_TOTAL_BYTES: usize = 32 * 1024;
pub(super) const MAX_DATABASE_PROFILES: usize = 32;

/// Trusted role preset selected by the coordinator, never a permission grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildRole {
    Analyst,
    Implementer,
    Reviewer,
}

/// The checkout confinement that a trusted host probe established for commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandCheckoutProfile {
    ReadOnlyCheckout,
    IsolatedPrivateCheckout,
}

/// Trusted host support that can only remove authority from a child view.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlatformSupport {
    pub workspace_edits: bool,
    pub command_profile: Option<CommandCheckoutProfile>,
    pub network_fetch: bool,
    pub database_reads: bool,
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
    pub write_paths: Vec<WorkspacePath>,
    pub capabilities: Capabilities,
    pub database_profiles: Vec<ProfileIdentity>,
}

/// Immutable root ceiling that a trusted coordinator narrows for one child.
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
    pub(super) write_paths: Vec<WorkspacePath>,
    pub(super) capabilities: Capabilities,
    pub(super) database_profiles: Vec<ProfileIdentity>,
}

/// Model-requested attenuation inputs. Construction validates only shape and bounds.
#[derive(Debug)]
pub struct ChildAuthorityRequest {
    pub(super) role: ChildRole,
    pub(super) write_paths: Vec<WorkspacePath>,
    pub(super) capabilities: Capabilities,
    pub(super) database_profiles: Vec<ProfileIdentity>,
}

/// The immutable, already-narrowed policy a future child factory receives.
#[derive(Debug)]
pub struct ChildAuthorityView {
    pub(super) job_id: JobId,
    pub(super) agent_id: AgentId,
    pub(super) invocation_id: InvocationId,
    pub(super) repository_id: RepositoryId,
    pub(super) snapshot_id: SnapshotId,
    pub(super) managed_checkout_id: Option<ManagedCheckoutId>,
    pub(super) policy_digest: Sha256Digest,
    pub(super) revocation_epoch: u64,
    pub(super) write_paths: Vec<WorkspacePath>,
    pub(super) capabilities: Capabilities,
    pub(super) database_profiles: Vec<ProfileIdentity>,
}

impl RootDelegationScope {
    pub fn new(input: RootDelegationInput) -> Result<Self, AuthorityError> {
        values::validate_paths(&input.write_paths)?;
        if !input.write_paths.is_empty() && input.managed_checkout_id.is_none() {
            return Err(AuthorityError::ManagedCheckoutRequired);
        }
        Ok(Self {
            job_id: input.job_id,
            agent_id: input.agent_id,
            invocation_id: input.invocation_id,
            repository_id: input.repository_id,
            snapshot_id: input.snapshot_id,
            managed_checkout_id: input.managed_checkout_id,
            policy_digest: input.policy_digest,
            revocation_epoch: input.revocation_epoch,
            write_paths: input.write_paths,
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
    pub fn new(
        role: ChildRole,
        write_paths: Vec<WorkspacePath>,
        capabilities: Capabilities,
        database_profiles: Vec<ProfileIdentity>,
    ) -> Result<Self, AuthorityError> {
        values::validate_paths(&write_paths)?;
        Ok(Self {
            role,
            write_paths,
            capabilities: values::validate_capabilities(&capabilities)?,
            database_profiles: values::validate_profiles(database_profiles)?,
        })
    }
}

/// Construction or shape failure before an authority view exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AuthorityError {
    #[error("authority path set exceeds its item bound")]
    TooManyWritePaths,
    #[error("authority path exceeds its byte bound")]
    WritePathTooLong,
    #[error("authority path set exceeds its aggregate byte bound")]
    WritePathsTooLarge,
    #[error("authority path set contains a duplicate")]
    DuplicateWritePath,
    #[error("authority paths require a managed checkout")]
    ManagedCheckoutRequired,
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
    #[error("program is outside child authority")]
    ProgramDenied,
    #[error("network destination is outside child authority")]
    NetworkDestinationDenied,
    #[error("database profile is outside child authority")]
    DatabaseProfileDenied,
}
