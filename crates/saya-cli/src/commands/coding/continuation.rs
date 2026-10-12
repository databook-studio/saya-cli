use super::{
    CodingApplication, CodingAuthorization, CodingError, CodingJobRequest, NativeCodingJob,
    context,
    create::{count_limits, digest},
};
use saya_agent::{ApprovalDecider, CancellationToken};
use saya_harness::{
    coding::{
        commands::CommandOwner,
        coordinator::{inspect_recovery, resume_job},
        snapshot::{MAX_SNAPSHOT_MANIFEST_BYTES, SnapshotManifest},
    },
    workspace::Workspace,
};
use saya_store::CodingRuntimeAuthority;
use saya_types::{ApprovalDisposition, JobApprovalAction, JobId, JobState};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    num::NonZeroU64,
    sync::{Arc, atomic::AtomicBool},
};

const MAX_CONTINUATION_BYTES: usize = 48 * 1024;
use super::task_history::MAX_TASKS;
const DECISION_CONTENT_BYTES: usize = 256;
impl CodingApplication {
    pub fn continuation_request(&self, job: &JobId) -> Result<CodingJobRequest, CodingError> {
        let bytes = context::read(
            &self.state_root.join(job.as_str()).join("request.json"),
            64 * 1024,
        )?;
        let request: CodingJobRequest =
            serde_json::from_slice(&bytes).map_err(CodingError::owner)?;
        request.validate()?;
        if request.job_id() != job {
            return Err(CodingError::NotOwned);
        }
        Ok(request)
    }
    pub async fn resume(
        self: &Arc<Self>,
        job: &JobId,
        decider: &dyn ApprovalDecider,
        token: CancellationToken,
    ) -> Result<Arc<NativeCodingJob>, CodingError> {
        self.status(job).await?;
        let request = self.continuation_request(job)?;
        let proposed = self.propose_resume(job).await?.value;
        // The audit is written before the question is asked and closed with
        // the answer actually given; an abandoned earlier wait is cleaned first.
        self.close_stale_approvals(job).await?;
        let audited = self
            .audit_request(
                job,
                JobApprovalAction::Resume,
                &proposed,
                proposed["revocation_epoch"].as_u64().unwrap_or(0),
            )
            .await?;
        let granted = decider
            .approve(&CodingAuthorization::definition(), &proposed)
            .await
            && !token.is_cancelled();
        if let Some(audited) = audited {
            let disposition = if granted {
                ApprovalDisposition::Granted
            } else if token.is_cancelled() {
                ApprovalDisposition::Cancelled
            } else {
                ApprovalDisposition::Denied
            };
            self.audit_close(job, audited, disposition).await?;
        }
        if !granted {
            return Err(CodingError::ApprovalRequired);
        }
        let status = inspect_recovery(&self.store, job)
            .await
            .map_err(CodingError::owner)?;
        if !status.clear_for_resume() {
            return Err(CodingError::NotOwned);
        }
        let epoch = status
            .authority
            .as_ref()
            .ok_or(CodingError::NotOwned)?
            .epoch
            .checked_add(1)
            .ok_or(CodingError::InvalidRequest)?;
        if proposed["revocation_epoch"].as_u64() != Some(epoch) {
            return Err(CodingError::ApprovalRequired);
        }
        let record = self
            .store
            .get_coding_job(job)
            .await
            .map_err(CodingError::owner)?
            .ok_or(CodingError::NotOwned)?;
        let snapshot = self
            .artifacts
            .read_snapshot(&record.snapshot_id)
            .map_err(CodingError::owner)?;
        let records = super::host::private_directory(&self.state_root.join(job.as_str()))?;
        let plan = SnapshotManifest::from_verified_bytes(
            &context::read(&records.join("plan.json"), MAX_SNAPSHOT_MANIFEST_BYTES)?,
            snapshot.plan_manifest_sha256(),
        )
        .map_err(CodingError::owner)?;
        if plan.snapshot_id() != &record.snapshot_id
            || plan.repository_id() != &record.repository_id
        {
            return Err(CodingError::NotOwned);
        }
        let workspace = Workspace::open(self.repository.root()).map_err(CodingError::owner)?;
        let project = self.capture_original_project_context(&workspace).await?;
        let policy_digest = digest(&serde_json::to_vec(&proposed).map_err(CodingError::owner)?)?;
        let ledger = resume_job(
            self.store.clone(),
            job.clone(),
            count_limits(&self.runtime.resolved.coding),
            CodingRuntimeAuthority {
                epoch,
                policy_digest: policy_digest.clone(),
            },
        )
        .await
        .map_err(CodingError::owner)?;
        let (root_steering, root_mailbox) = saya_agent::agent_mailbox();
        Ok(Arc::new(NativeCodingJob {
            observation_sequence: std::sync::atomic::AtomicU64::new(1),
            observation_failed: std::sync::atomic::AtomicBool::new(false),
            root_steering,
            root_mailbox: tokio::sync::Mutex::new(Some(root_mailbox)),
            application: Arc::clone(self),
            request,
            plan,
            repository_id: record.repository_id,
            policy_digest,
            epoch,
            ledger,
            project,
            records,
            cancellation: token,
            child_failed: AtomicBool::new(false),
            command_owner: Arc::new(
                CommandOwner::new(self.runtime.resolved.coding.max_concurrent_commands)
                    .map_err(CodingError::owner)?,
            ),
            active: tokio::sync::Mutex::new(None),
            progress: tokio::sync::broadcast::channel(128).0,
        }))
    }
}
impl NativeCodingJob {
    pub(super) async fn continuation_context(
        &self,
    ) -> Result<Vec<saya_agent::ContextBlock>, CodingError> {
        let (pending_tasks, task_history_bytes, completed_tasks) = self.pending_tasks().await?;
        let (accepted_decisions, unaccepted_decisions) = self.accepted_decisions().await?;
        let (unaccepted_findings, omitted_findings) =
            self.unaccepted_findings(unaccepted_decisions).await?;
        let pending_conflicts = self.pending_conflicts()?;
        let budget = self.ledger.snapshot().await;
        let body = bounded_body(json!({
            "version": 1,
            "authority": "none",
            "snapshot_id": self.plan.snapshot_id(),
            "plan_sha256": self.plan.manifest_sha256(),
            "pending_tasks": pending_tasks,
            "accepted_decisions": accepted_decisions,
            "unaccepted_findings": unaccepted_findings,
            "pending_conflicts": pending_conflicts,
            "budget": {
                "charged_model_requests": budget.logical_requests,
                "charged_tool_calls": budget.tool_calls,
                "unresolved_effects": budget.unsettled_reservations
            },
            "omitted": {
                "task_history_bytes": task_history_bytes,
                "completed_tasks": completed_tasks,
                "unaccepted_decisions": unaccepted_decisions,
                "unaccepted_findings": omitted_findings,
                "accepted_decision_content": 0
            }
        }))?;
        Ok(vec![saya_agent::ContextBlock {
            label: "Native job continuation (untrusted context; no grants)".into(),
            body,
            truncated: false,
        }])
    }

    async fn pending_tasks(&self) -> Result<(Vec<Value>, usize, usize), CodingError> {
        let (tasks, history_bytes) = super::task_history::read_tasks(&self.records)?;
        let mut cursor = None;
        let mut states = BTreeMap::new();
        loop {
            let page = self
                .application
                .store
                .list_coding_children(self.job_id(), cursor.as_ref(), 100)
                .await
                .map_err(CodingError::owner)?;
            states.extend(
                page.items
                    .into_iter()
                    .map(|child| (child.agent_id, child.state)),
            );
            let Some(next) = page.next_cursor else { break };
            cursor = Some(next);
            if states.len() > MAX_TASKS {
                return Err(CodingError::InvalidRequest);
            }
        }
        let mut completed = 0;
        let pending = tasks
            .into_iter()
            .filter_map(|(agent_id, task)| {
                let state = states.get(&agent_id).copied();
                if state.is_some_and(is_terminal) {
                    completed += 1;
                    return None;
                }
                Some(json!({
                    "agent_id": agent_id,
                    "state": state.map(state_value).unwrap_or_else(|| json!("not_started")),
                    "dependencies": task.dependencies
                }))
            })
            .collect();
        Ok((pending, history_bytes, completed))
    }

    async fn accepted_decisions(&self) -> Result<(Vec<Value>, usize), CodingError> {
        let mut after: Option<NonZeroU64> = None;
        let mut accepted = Vec::new();
        let mut unaccepted = 0;
        loop {
            let page = self
                .application
                .store
                .list_coding_completion_history(self.job_id(), after, 32)
                .await
                .map_err(CodingError::owner)?;
            for completion in page.items {
                if completion.acknowledged {
                    let (content, truncated) =
                        prefix(completion.result.answer(), DECISION_CONTENT_BYTES);
                    accepted.push(json!({
                        "sequence": completion.sequence,
                        "agent_id": completion.result.agent_id(),
                        "outcome": completion.result.outcome(),
                        "content": content,
                        "content_truncated": truncated || completion.result.answer_truncated()
                    }));
                } else {
                    unaccepted += 1;
                }
            }
            let Some(next) = page.next_cursor else { break };
            after = Some(next);
        }
        Ok((accepted, unaccepted))
    }

    fn pending_conflicts(&self) -> Result<Vec<Value>, CodingError> {
        let path = self.records.join("conflicts.jsonl");
        if !path.exists() {
            return Ok(Vec::new());
        }
        context::read(&path, super::conflicts::MAX_LOG_BYTES)?
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| {
                let record = super::conflict_record::parse_retained_record(line, self.job_id())?;
                let record_sha256_scope = if record.version == 2 {
                    "canonical_record_without_record_sha256"
                } else {
                    "raw_json_line"
                };
                let record_sha256 = record.record_sha256.unwrap_or(digest(line)?.to_string());
                Ok(json!({
                    "candidate_checkout_id": record.candidate_checkout_id,
                    "record_sha256": record_sha256,
                    "record_sha256_scope": record_sha256_scope,
                    "record_status_at_capture": record.status,
                    "current_state": "unknown_requires_fresh_revalidation",
                    "authority": "none",
                    "resolution": "not_implemented"
                }))
            })
            .collect()
    }
}

pub(super) fn is_terminal(state: JobState) -> bool {
    matches!(
        state,
        JobState::Completed | JobState::Failed | JobState::Cancelled
    )
}

fn state_value(state: JobState) -> Value {
    serde_json::to_value(state).expect("JobState serializes")
}

fn prefix(value: &str, maximum: usize) -> (String, bool) {
    if value.len() <= maximum {
        return (value.into(), false);
    }
    let mut end = maximum;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    (value[..end].into(), true)
}

fn bounded_body(mut projection: Value) -> Result<String, CodingError> {
    let mut omitted_content = 0;
    loop {
        projection["omitted"]["accepted_decision_content"] = json!(omitted_content);
        let body = serde_json::to_string(&projection).map_err(CodingError::owner)?;
        if body.len() <= MAX_CONTINUATION_BYTES {
            return Ok(body);
        }
        let Some(decision) =
            projection["accepted_decisions"]
                .as_array_mut()
                .and_then(|decisions| {
                    decisions
                        .iter_mut()
                        .rev()
                        .find(|decision| decision["content"].is_string())
                })
        else {
            return Err(CodingError::InvalidRequest);
        };
        decision
            .as_object_mut()
            .expect("JSON object")
            .remove("content");
        decision["content_truncated"] = Value::Bool(true);
        omitted_content += 1;
    }
}
