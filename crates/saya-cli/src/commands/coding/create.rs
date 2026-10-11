use super::{
    CodingApplication, CodingAuthorization, CodingError, CodingJobRequest, NativeCodingJob,
    host::private_directory,
};
use saya_agent::{ApprovalDecider, CancellationToken};
use saya_harness::{
    coding::{
        budget::{CountLimits, JobBudgetLedger},
        context::OriginalProjectContext,
    },
    workspace::Workspace,
};
use saya_store::NewCodingJob;
use saya_types::{CodingEventKind, CodingEventV1, JobState, Sha256Digest};
use sha2::{Digest, Sha256};
use std::{num::NonZeroU64, sync::Arc, time::Duration};

impl CodingApplication {
    /// Captures the original project only after exact fresh approval; creates
    /// the durable job and budget owner once. Model text supplies no grants.
    pub async fn create(
        self: &Arc<Self>,
        request: CodingJobRequest,
        decider: &dyn ApprovalDecider,
        token: CancellationToken,
    ) -> Result<Arc<NativeCodingJob>, CodingError> {
        if !self.runtime.resolved.coding.enabled {
            return Err(CodingError::Disabled);
        }
        request.validate()?;
        let plan = self.plan_request(&request, &token).await?;
        let proposed = self.proposal(&request, &plan, 1, "start")?.value;
        if !decider
            .approve(&CodingAuthorization::definition(), &proposed)
            .await
            || token.is_cancelled()
        {
            return Err(CodingError::ApprovalRequired);
        }
        let workspace = Workspace::open(self.repository.root()).map_err(CodingError::owner)?;
        let captured = self
            .service
            .capture_snapshot(&self.repository, &workspace, &plan, &self.artifacts, &token)
            .await
            .map_err(CodingError::owner)?;
        let project = OriginalProjectContext::capture(&workspace, None, vec![])
            .map_err(CodingError::owner)?;
        let repository_id = captured.repository_id().clone();
        self.store
            .create_coding_job(NewCodingJob {
                job_id: request.job_id.clone(),
                repository_id: repository_id.clone(),
                snapshot_id: plan.snapshot_id().clone(),
            })
            .await
            .map_err(CodingError::owner)?;
        self.store
            .append_coding_event(
                CodingEventV1::new(
                    NonZeroU64::MIN,
                    request.job_id.clone(),
                    None,
                    None,
                    CodingEventKind::StateChanged {
                        state: JobState::Running,
                    },
                )
                .map_err(CodingError::owner)?,
            )
            .await
            .map_err(CodingError::owner)?;
        let policy_digest = digest(&serde_json::to_vec(&proposed).map_err(CodingError::owner)?)?;
        let ledger = JobBudgetLedger::new(
            self.store.clone(),
            request.job_id.clone(),
            count_limits(&self.runtime.resolved.coding),
        )
        .await
        .map_err(CodingError::owner)?;
        ledger
            .bind_runtime(1, &policy_digest)
            .await
            .map_err(CodingError::owner)?;
        let records = private_directory(&self.state_root.join(request.job_id.as_str()))?;
        super::context::write_new(
            &records.join("request.json"),
            &serde_json::to_vec(&request).map_err(CodingError::owner)?,
            64 * 1024,
        )?;
        super::context::write_new(
            &records.join("plan.json"),
            plan.canonical_bytes(),
            saya_harness::coding::snapshot::MAX_SNAPSHOT_MANIFEST_BYTES,
        )?;
        let (root_steering, root_mailbox) = saya_agent::agent_mailbox();
        Ok(Arc::new(NativeCodingJob {
            observation_sequence: std::sync::atomic::AtomicU64::new(1),
            observation_failed: std::sync::atomic::AtomicBool::new(false),
            root_steering,
            root_mailbox: tokio::sync::Mutex::new(Some(root_mailbox)),
            child_failed: std::sync::atomic::AtomicBool::new(false),
            command_owner: Arc::new(
                saya_harness::coding::commands::CommandOwner::new(
                    self.runtime.resolved.coding.max_concurrent_commands,
                )
                .map_err(CodingError::owner)?,
            ),
            active: tokio::sync::Mutex::new(None),
            progress: tokio::sync::broadcast::channel(128).0,
            application: Arc::clone(self),
            request,
            plan,
            repository_id,
            policy_digest,
            epoch: 1,
            ledger,
            project,
            records,
            cancellation: token,
        }))
    }
}

pub(super) fn count_limits(config: &saya_config::ResolvedCoding) -> CountLimits {
    CountLimits {
        logical_requests: config.max_logical_model_requests as u64,
        transport_attempts: config.max_physical_transport_attempts as u64,
        tool_calls: config.max_tool_calls as u64,
        model_slots: config.max_concurrent_model_requests as u64,
        child_slots: config.max_children as u64,
        deadline: Duration::from_secs(config.max_elapsed_seconds),
    }
}
pub(super) fn digest(bytes: &[u8]) -> Result<Sha256Digest, CodingError> {
    let digest = Sha256::digest(bytes);
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    Sha256Digest::parse(&hex).map_err(CodingError::owner)
}
