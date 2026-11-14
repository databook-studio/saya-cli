use saya_types::{
    AgentId, ApprovalNetworkDestination, ApprovalNetworkScheme, Capabilities, InvocationId, JobId,
    ManagedCheckoutId, ProfileIdentity, RepositoryId, Sha256Digest, SnapshotId,
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

    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    pub fn database_profiles(&self) -> &[ProfileIdentity] {
        &self.database_profiles
    }

    /// Whether this view may write its working checkout at all: the write
    /// capability the trusted coordinator narrowed onto it.
    pub fn workspace_write(&self) -> bool {
        self.capabilities.workspace_write
    }

    /// Admits one write in `checkout`.
    ///
    /// The view's checkout is the shared working checkout W, and a write-capable
    /// view admits **any** path inside it — `.git` included. There is no
    /// job-level write-path ceiling; the operator's review and apply of W is
    /// the check. The only surviving bounds are the checkout identity (a view
    /// narrowed for another checkout refuses) and the capability itself.
    pub fn admit_workspace_write(
        &self,
        checkout: &ManagedCheckoutId,
    ) -> Result<(), AuthorityRefusal> {
        let Some(expected) = self.managed_checkout_id() else {
            return Err(AuthorityRefusal::ManagedCheckoutUnavailable);
        };
        if checkout != expected {
            return Err(AuthorityRefusal::ManagedCheckoutMismatch);
        }
        if !self.workspace_write() {
            return Err(AuthorityRefusal::WorkspaceWriteDenied);
        }
        Ok(())
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
