use saya_types::{
    AgentId, ApprovalNetworkDestination, ApprovalNetworkScheme, Capabilities, InvocationId, JobId,
    ManagedCheckoutId, ProfileIdentity, RepositoryId, Sha256Digest, SnapshotId, WorkspacePath,
};

use super::authority::{AuthorityRefusal, ChildAuthorityView};

impl ChildAuthorityView {
    pub fn job_id(&self) -> &JobId {
        &self.job_id
    }

    pub fn agent_id(&self) -> &AgentId {
        &self.agent_id
    }

    pub fn invocation_id(&self) -> &InvocationId {
        &self.invocation_id
    }

    pub fn repository_id(&self) -> &RepositoryId {
        &self.repository_id
    }

    pub fn snapshot_id(&self) -> &SnapshotId {
        &self.snapshot_id
    }

    pub fn managed_checkout_id(&self) -> Option<&ManagedCheckoutId> {
        self.managed_checkout_id.as_ref()
    }

    pub fn policy_digest(&self) -> &Sha256Digest {
        &self.policy_digest
    }

    pub fn revocation_epoch(&self) -> u64 {
        self.revocation_epoch
    }

    pub fn write_paths(&self) -> &[WorkspacePath] {
        &self.write_paths
    }

    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    pub fn database_profiles(&self) -> &[ProfileIdentity] {
        &self.database_profiles
    }

    pub fn admit_workspace_write(
        &self,
        checkout: &ManagedCheckoutId,
        path: &WorkspacePath,
    ) -> Result<(), AuthorityRefusal> {
        let Some(expected) = self.managed_checkout_id() else {
            return Err(AuthorityRefusal::ManagedCheckoutUnavailable);
        };
        if checkout != expected {
            return Err(AuthorityRefusal::ManagedCheckoutMismatch);
        }
        self.write_paths
            .contains(path)
            .then_some(())
            .ok_or(AuthorityRefusal::WorkspaceWriteDenied)
    }

    pub fn admit_runner_program(&self, program: &str) -> Result<(), AuthorityRefusal> {
        self.capabilities
            .runner
            .as_ref()
            .is_some_and(|scope| scope.programs.iter().any(|allowed| allowed == program))
            .then_some(())
            .ok_or(AuthorityRefusal::ProgramDenied)
    }

    pub fn admit_interpreter_program(&self, program: &str) -> Result<(), AuthorityRefusal> {
        self.capabilities
            .interpreter
            .as_ref()
            .is_some_and(|scope| scope.programs.iter().any(|allowed| allowed == program))
            .then_some(())
            .ok_or(AuthorityRefusal::ProgramDenied)
    }

    pub fn admit_network_destination(
        &self,
        destination: &ApprovalNetworkDestination,
    ) -> Result<(), AuthorityRefusal> {
        let scheme = match destination.scheme() {
            ApprovalNetworkScheme::Http => "http",
            ApprovalNetworkScheme::Https => "https",
            _ => return Err(AuthorityRefusal::NetworkDestinationDenied),
        };
        self.capabilities
            .fetch
            .as_ref()
            .is_some_and(|scope| {
                scope
                    .destinations
                    .iter()
                    .any(|allowed| allowed.scheme == scheme && allowed.host == destination.host())
            })
            .then_some(())
            .ok_or(AuthorityRefusal::NetworkDestinationDenied)
    }

    pub fn admit_database_profile(
        &self,
        profile: &ProfileIdentity,
    ) -> Result<(), AuthorityRefusal> {
        self.database_profiles
            .contains(profile)
            .then_some(())
            .ok_or(AuthorityRefusal::DatabaseProfileDenied)
    }
}
