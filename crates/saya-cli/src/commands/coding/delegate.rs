use super::{CodingError, NativeCodingJob, approval::ChildIssuanceApproval, create::digest};
use crate::agent::tools::child::{ChildComposeRequest, compose_child};
use saya_harness::coding::{
    context::{ChildBrief, ChildProjectionOptions},
    coordinator::ScheduledChild,
    integration::AssignedPaths,
    policy::{ChildAuthorityRequest, ChildRole, RootDelegationInput, RootDelegationScope},
    worktree::{
        CreateCheckoutInput, OwnershipInput, create_managed_checkout, verify_owned_checkout,
    },
};
use saya_store::NewCodingChild;
use saya_types::{
    AgentId, Capabilities, InvocationId, ManagedCheckoutId, RunnerScope, WorkspacePath,
};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingTask {
    pub agent_id: AgentId,
    pub task: String,
    #[serde(default)]
    pub write_paths: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<AgentId>,
}
impl NativeCodingJob {
    pub(super) async fn prepare_child(
        &self,
        task: CodingTask,
    ) -> Result<ScheduledChild, CodingError> {
        if task.agent_id.as_str() == "coordinator"
            || task.task.trim().is_empty()
            || task.task.len() > 16 * 1024
            || task.dependencies.len() > 8
        {
            return Err(CodingError::InvalidRequest);
        }
        let paths = if task.write_paths.is_empty() {
            None
        } else {
            Some(AssignedPaths::parse(task.write_paths.clone()).map_err(CodingError::owner)?)
        };
        let ceiling = if self.request.write_paths.is_empty() {
            None
        } else {
            Some(
                AssignedPaths::parse(self.request.write_paths.clone())
                    .map_err(CodingError::owner)?,
            )
        };
        if task.write_paths.iter().any(|path| {
            !ceiling
                .as_ref()
                .is_some_and(|ceiling| ceiling.contains(path))
        }) {
            return Err(CodingError::ApprovalRequired);
        }
        let identity =
            digest(format!("{}:{}", self.job_id().as_str(), task.agent_id.as_str()).as_bytes())?;
        let checkout = ManagedCheckoutId::parse(&format!("child-{}", &identity.as_str()[..32]))
            .map_err(CodingError::owner)?;
        let invocation = InvocationId::parse(&format!("inv-{}", &identity.as_str()[..32]))
            .map_err(CodingError::owner)?;
        let host = &self.application;
        host.store
            .create_coding_child(NewCodingChild {
                job_id: self.job_id().clone(),
                agent_id: task.agent_id.clone(),
                current_invocation_id: invocation.clone(),
            })
            .await
            .map_err(CodingError::owner)?;
        create_managed_checkout(
            &CreateCheckoutInput {
                store: &host.store,
                sequencer: &self.ledger.sequencer(),
                service: &host.service,
                repository: &host.repository,
                artifacts: &host.artifacts,
                plan: &self.plan,
                job_id: self.job_id(),
                agent_id: Some(task.agent_id.clone()),
                checkout_id: &checkout,
                base_commit: self.plan.base_commit(),
            },
            &self.cancellation,
        )
        .await
        .map_err(CodingError::owner)?;
        let ownership = OwnershipInput {
            store: &host.store,
            service: &host.service,
            repository: &host.repository,
            artifacts: &host.artifacts,
            job_id: self.job_id(),
            checkout_id: &checkout,
        };
        let before = verify_owned_checkout(&ownership, &self.cancellation)
            .await
            .map_err(CodingError::owner)?;
        let capabilities = Capabilities {
            workspace_write: !task.write_paths.is_empty(),
            runner: if self.request.programs.is_empty() {
                None
            } else {
                Some(RunnerScope::new(self.request.programs.clone()).map_err(CodingError::owner)?)
            },
            ..Capabilities::default()
        };
        let write_paths = task
            .write_paths
            .iter()
            .map(|path| WorkspacePath::try_from(path.clone()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(CodingError::owner)?;
        let input = RootDelegationInput {
            job_id: self.job_id().clone(),
            agent_id: task.agent_id,
            invocation_id: invocation,
            repository_id: self.repository_id.clone(),
            snapshot_id: self.plan.snapshot_id().clone(),
            managed_checkout_id: Some(checkout.clone()),
            policy_digest: self.policy_digest.clone(),
            revocation_epoch: self.epoch,
            write_paths: write_paths.clone(),
            capabilities: capabilities.clone(),
            database_profiles: vec![],
        };
        let approval = ChildIssuanceApproval {
            expected: input.approval_request(),
        };
        let root = RootDelegationScope::approve(input, &approval)
            .await
            .map_err(CodingError::owner)?;
        let after = verify_owned_checkout(&ownership, &self.cancellation)
            .await
            .map_err(CodingError::owner)?;
        if before.path() != after.path()
            || before.base_commit() != after.base_commit()
            || before.snapshot_id() != after.snapshot_id()
            || before.snapshot_sha256() != after.snapshot_sha256()
            || before.plan_sha256() != after.plan_sha256()
            || before.agent_id() != after.agent_id()
        {
            return Err(CodingError::NotOwned);
        }
        let config = &host.runtime.resolved.coding;
        let options = ChildProjectionOptions {
            model: host
                .runtime
                .resolved
                .endpoints
                .get(saya_config::ORCHESTRATOR_ROLE)
                .ok_or(CodingError::InvalidRequest)?
                .model
                .clone(),
            profile_names: vec![],
            agent_limits: super::parent::agent_limits(config),
            answering_limits: super::parent::answering_limits(config),
            request_preflight: None,
        };
        let composition = compose_child(ChildComposeRequest {
            ownership,
            cancellation: &self.cancellation,
            root: &root,
            authority: ChildAuthorityRequest::new(
                ChildRole::Implementer,
                write_paths,
                capabilities,
                vec![],
            )
            .map_err(CodingError::owner)?,
            project: &self.project,
            brief: ChildBrief {
                task: task.task,
                dependencies: vec![],
                artifacts: vec![],
            },
            options,
            runtime: &host.runtime,
            program_dir: &host.program_dir,
            trusted_record_dir: &self.records,
            command_timeout: Duration::from_secs(60),
        })
        .await
        .map_err(CodingError::owner)?;
        let provider = host
            .runtime
            .resolved
            .endpoints
            .get(saya_config::ORCHESTRATOR_ROLE)
            .ok_or(CodingError::InvalidRequest)?
            .provider
            .as_str()
            .to_owned();
        let coding = Arc::new(super::checks::ChildCodingOperations {
            application: Arc::clone(host),
            job: self.job_id().clone(),
            checkout,
            authority: composition.spec.authority().clone(),
            owner: Arc::clone(&self.command_owner),
            ledger: self.ledger.clone(),
            records: self.records.clone(),
            failed: std::sync::atomic::AtomicBool::new(false),
            sessions: tokio::sync::Mutex::new(Vec::new()),
        });
        ScheduledChild::new(
            composition.spec,
            task.dependencies,
            paths,
            provider,
            Arc::new(composition.factory.with_coding(coding)),
        )
        .map_err(CodingError::owner)
    }
}
