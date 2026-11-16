//! The direct-mode root's own workspace composition: composes the child
//! workspace toolset over the agent-less working checkout W (from the same
//! authority path a child uses) with the coordinator's optional delegation
//! tools. Lives in `commands::coding` so it can read the job's private fields;
//! `agent::tools::coding::direct::RootTools::build` is the caller-facing front.
//!
//! The root's coding operations (commands, checks) bind the narrowed authority
//! view: `compose_root_workspace` narrows the view first, then calls the
//! `coding` closure with it, so the `ChildCodingOperations` carry the same
//! view the toolset is built over — no double approve, no circular dependency.

use std::sync::Arc;

use saya_harness::coding::policy::{
    ChildAuthorityRequest, ChildAuthorityView, ChildRole, RootDelegationInput,
};
use saya_harness::coding::worktree::{VerifiedCheckout, verify_in_place_root};
use saya_types::{AgentId, Capabilities, InvocationId};

use super::{CodingError, NativeCodingJob, approval::ChildIssuanceApproval};
use crate::agent::tools::child::{
    CheckoutProof, CodingOpsFactory, RootComposeRequest, RootWorkspace, compose_root_workspace,
};
use crate::commands::coding::checks::ChildCodingOperations;

impl NativeCodingJob {
    /// Composes the direct-mode root's workspace toolset over the agent-less
    /// working checkout W: the same workspace tools a child gets over W, plus
    /// the root's own coding operations (commands, checks) bound to the
    /// narrowed authority view. The returned toolset carries the executor bound
    /// to W, the definitions the loop advertises, the exact-write decider, and
    /// the narrowed view. Databases are attached to the workspace (the root
    /// queries like a child), never to the coordinator.
    pub(crate) async fn compose_direct_workspace(
        &self,
        checkout: &saya_types::ManagedCheckoutId,
        verified: Option<Arc<VerifiedCheckout>>,
    ) -> Result<RootWorkspace, CodingError> {
        let proof = CheckoutProof::Managed(self.ownership(checkout)?);
        self.compose_workspace(checkout, proof, verified).await
    }

    /// Composes the same workspace toolset over an in-place job's own folder.
    /// The folder is verified from the job's private baseline record; there is
    /// no managed checkout and no Git history, so the Git-only coding
    /// operations (commands, checks) are not attached.
    pub(crate) async fn compose_in_place_workspace(&self) -> Result<RootWorkspace, CodingError> {
        let record = self.in_place_record()?;
        let input = self.in_place_input(&record);
        let checkout = verify_in_place_root(&input)
            .map_err(CodingError::owner)?
            .checkout_id()
            .clone();
        self.compose_workspace(&checkout, CheckoutProof::InPlace(input), None)
            .await
    }

    async fn compose_workspace(
        &self,
        checkout: &saya_types::ManagedCheckoutId,
        proof: CheckoutProof<'_>,
        verified: Option<Arc<VerifiedCheckout>>,
    ) -> Result<RootWorkspace, CodingError> {
        let cancellation = self.cancellation.clone();
        // The root of the working checkout writes it: there is no job-level
        // write-path ceiling, so the grant is a role fact, not a path list.
        let mut capabilities = Capabilities::default();
        capabilities.workspace_write = true;
        let identity =
            super::create::digest(format!("{}:root", self.job_id().as_str()).as_bytes())?;
        let input = RootDelegationInput {
            job_id: self.job_id().clone(),
            agent_id: AgentId::parse("coordinator").map_err(CodingError::owner)?,
            invocation_id: InvocationId::parse(&format!("inv-{}", &identity.as_str()[..32]))
                .map_err(CodingError::owner)?,
            repository_id: self.repository_id.clone(),
            snapshot_id: self.plan.snapshot_id().clone(),
            managed_checkout_id: Some(checkout.clone()),
            policy_digest: self.policy_digest.clone(),
            revocation_epoch: self.epoch,
            capabilities: capabilities.clone(),
            database_profiles: self.request.database_identities.values().cloned().collect(),
        };
        let approval = ChildIssuanceApproval {
            expected: input.approval_request(),
        };
        let authority = ChildAuthorityRequest::new(
            ChildRole::Implementer,
            capabilities,
            self.request.database_identities.values().cloned().collect(),
        )
        .map_err(CodingError::owner)?;
        // The coding operations bind the narrowed view; capture owned clones so
        // the closure is Send and borrows nothing from `self`.
        let app = Arc::clone(&self.application);
        let job_id = self.job_id().clone();
        let checkout_id = checkout.clone();
        let ledger = self.ledger.clone();
        let records = self.records.clone();
        let before_images = self.before_images();
        // The coding operations are attached in both modes: they carry the
        // `delete` tool and the private search prefixes, which an in-place
        // root needs as much as a managed checkout. The operations themselves
        // resolve an in-place root from its baseline record and never touch
        // Git there.
        let coding: Option<CodingOpsFactory<'static>> =
            Some(Box::new(move |view: &ChildAuthorityView| {
                Arc::new(ChildCodingOperations {
                    application: app,
                    job: job_id,
                    checkout: checkout_id,
                    authority: view.clone(),
                    ledger,
                    records,
                    before_images,
                    owner_failure: Arc::new(super::NativeOwnerFailureState::default()),
                    failed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                })
            }));
        let request = RootComposeRequest {
            proof,
            verified,
            cancellation: &cancellation,
            // Only an in-place folder has no apply gate, so only there is the
            // stated `--write` scope the tool boundary's path ceiling. A
            // managed checkout is itself the ceiling.
            write_scope: if self.application.is_in_place() {
                self.request.assigned_paths()?
            } else {
                None
            },
            root: input,
            decider: &approval,
            authority,
            runtime: &self.application.runtime,
            coding,
            databases: self.databases(),
            // The job's own set, so a resumed job narrows its workspace member
            // exactly as the job it resumed did.
            tools: self.tools.clone(),
            // The same directory the job's shell spools into, so `read`/`grep`
            // reach exactly this job's saved output.
            spool_dir: Some(crate::agent::tools::shell_policy::spool_dir(self.records())),
        };
        compose_root_workspace(request)
            .await
            .map_err(CodingError::owner)
    }
}
