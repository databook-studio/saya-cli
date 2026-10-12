mod network;
mod validated;

pub use network::{ApprovalNetworkDestination, ApprovalNetworkScheme};
pub use validated::{MemoryKey, SafeCommandReference, WorkingDirectory, WorkspacePath};

pub use super::approval_job_scope::JobApprovalAction;
use super::{ManagedCheckoutId, RepositoryId, Sha256Digest};
use crate::ProfileIdentity;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// One exact pending operation and its bounded resource. This shape carries
/// consent context only; a runtime must still check live root authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum ApprovalScope {
    WorkspaceWrite {
        managed_checkout_id: ManagedCheckoutId,
        relative_path: WorkspacePath,
    },
    ProcessExecution {
        managed_checkout_id: ManagedCheckoutId,
        command_reference: SafeCommandReference,
        argv_sha256: Sha256Digest,
        cwd: WorkingDirectory,
    },
    NetworkAccess {
        destination: ApprovalNetworkDestination,
    },
    DatabaseRead {
        profile_identity: ProfileIdentity,
        statement_sha256: Sha256Digest,
    },
    MemoryWrite {
        repository_id: RepositoryId,
        memory_key: MemoryKey,
    },
    RepositoryMutation {
        managed_checkout_id: ManagedCheckoutId,
        action: RepositoryMutationAction,
    },
    /// A job-level operation approved against one exact proposal digest.
    JobOperation {
        action: JobApprovalAction,
        proposal_sha256: Sha256Digest,
    },
}

impl ApprovalScope {
    pub fn workspace_write(
        managed_checkout_id: ManagedCheckoutId,
        relative_path: impl Into<String>,
    ) -> Result<Self, ApprovalScopeError> {
        Ok(Self::WorkspaceWrite {
            managed_checkout_id,
            relative_path: WorkspacePath::new(relative_path.into())?,
        })
    }

    pub fn process_execution(
        managed_checkout_id: ManagedCheckoutId,
        command_reference: impl Into<String>,
        argv_sha256: Sha256Digest,
        cwd: impl Into<String>,
    ) -> Result<Self, ApprovalScopeError> {
        Ok(Self::ProcessExecution {
            managed_checkout_id,
            command_reference: SafeCommandReference::new(command_reference.into())?,
            argv_sha256,
            cwd: WorkingDirectory::new(cwd.into())?,
        })
    }

    pub fn network_access(destination: ApprovalNetworkDestination) -> Self {
        Self::NetworkAccess { destination }
    }

    pub fn database_read(
        profile_identity: ProfileIdentity,
        statement_sha256: Sha256Digest,
    ) -> Self {
        Self::DatabaseRead {
            profile_identity,
            statement_sha256,
        }
    }

    pub fn memory_write(
        repository_id: RepositoryId,
        memory_key: impl Into<String>,
    ) -> Result<Self, ApprovalScopeError> {
        Ok(Self::MemoryWrite {
            repository_id,
            memory_key: MemoryKey::new(memory_key.into())?,
        })
    }

    pub fn repository_mutation(
        managed_checkout_id: ManagedCheckoutId,
        action: RepositoryMutationAction,
    ) -> Self {
        Self::RepositoryMutation {
            managed_checkout_id,
            action,
        }
    }

    pub fn job_operation(action: JobApprovalAction, proposal_sha256: Sha256Digest) -> Self {
        Self::JobOperation {
            action,
            proposal_sha256,
        }
    }
}

/// A finite mutation request; it never authorizes Git by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RepositoryMutationAction {
    CreateCheckout,
    RemoveCheckout,
    IntegrateCandidate,
    ApplyBack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ApprovalScopeError {
    #[error("approval scope contains an invalid path")]
    InvalidPath,
    #[error("approval command reference is not a safe reference")]
    InvalidCommandReference,
    #[error("approval memory key is not a safe key")]
    InvalidMemoryKey,
    #[error("approval network destination is not a supported host-only destination")]
    InvalidDestination,
}
