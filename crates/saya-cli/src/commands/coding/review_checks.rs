use super::{CodingError, NativeCodingJob, approval::ChildIssuanceApproval, review::ReviewCheck};
use saya_harness::coding::{
    checks::{CheckExecutionInput, execute_check, execute_check_with_diagnostics},
    policy::{ChildAuthorityRequest, ChildRole, RootDelegationInput, RootDelegationScope},
    worktree::{confine_checkout, verify_owned_checkout},
};
use saya_types::{AgentId, Capabilities, InvocationId, ManagedCheckoutId, RunnerScope};
use std::sync::Arc;
impl NativeCodingJob {
    pub(super) async fn candidate_checks(
        &self,
        checkout: &ManagedCheckoutId,
    ) -> Result<Vec<ReviewCheck>, CodingError> {
        let profiles = &self.application.runtime.resolved.coding.check_profiles;
        if profiles.is_empty() {
            return Ok(Vec::new());
        }
        let ownership = self.ownership(checkout);
        let verified = verify_owned_checkout(&ownership, &self.cancellation)
            .await
            .map_err(CodingError::owner)?;
        let capabilities = Capabilities {
            runner: if self.request.programs.is_empty() {
                None
            } else {
                Some(RunnerScope::new(self.request.programs.clone()).map_err(CodingError::owner)?)
            },
            ..Capabilities::default()
        };
        let input = RootDelegationInput {
            job_id: self.job_id().clone(),
            agent_id: AgentId::parse("coordinator-check").map_err(CodingError::owner)?,
            invocation_id: InvocationId::parse("candidate-check").map_err(CodingError::owner)?,
            repository_id: self.repository_id.clone(),
            snapshot_id: self.plan.snapshot_id().clone(),
            managed_checkout_id: Some(checkout.clone()),
            policy_digest: self.policy_digest.clone(),
            revocation_epoch: self.epoch,
            write_paths: vec![],
            capabilities: capabilities.clone(),
            database_profiles: vec![],
        };
        let approval = ChildIssuanceApproval {
            expected: input.approval_request(),
        };
        let root = RootDelegationScope::approve(input, &approval)
            .await
            .map_err(CodingError::owner)?;
        let fresh = verify_owned_checkout(&ownership, &self.cancellation)
            .await
            .map_err(CodingError::owner)?;
        if fresh.path() != verified.path()
            || fresh.snapshot_sha256() != verified.snapshot_sha256()
            || fresh.plan_sha256() != verified.plan_sha256()
            || fresh.base_commit() != verified.base_commit()
            || fresh.agent_id() != verified.agent_id()
        {
            return Err(CodingError::NotOwned);
        }
        let confined =
            confine_checkout(&fresh, &self.application.program_dir).map_err(CodingError::owner)?;
        let authority = root
            .narrow(
                ChildAuthorityRequest::new(ChildRole::Implementer, vec![], capabilities, vec![])
                    .map_err(CodingError::owner)?,
                confined.platform_support(),
            )
            .map_err(CodingError::owner)?;
        let mut results = Vec::new();
        for profile in profiles.values() {
            let invocation = super::checks::invocation(profile)?;
            let input = CheckExecutionInput {
                ownership: &ownership,
                authority: &authority,
                invocation: &invocation,
                owner: &self.command_owner,
                cancellation: &self.cancellation,
                program_dir: &self.application.program_dir,
                output_root: &self.records,
                resolver: Arc::new(saya_harness::runner::StaticCredentialSource::new([])),
            };
            results.push(if invocation.diagnostic_format().is_some() {
                ReviewCheck::Diagnostics(
                    execute_check_with_diagnostics(input)
                        .await
                        .map_err(CodingError::owner)?,
                )
            } else {
                ReviewCheck::Plain(execute_check(input).await.map_err(CodingError::owner)?)
            });
        }
        Ok(results)
    }
}
