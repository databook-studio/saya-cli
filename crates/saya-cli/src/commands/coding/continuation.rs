use super::{
    CodingApplication, CodingAuthorization, CodingError, CodingJobRequest, NativeCodingJob,
    context, create::digest, limits,
};
use assembly::bounded_body;
use saya_agent::{ApprovalDecider, CancellationToken};
use saya_harness::{
    coding::coordinator::{inspect_recovery, resume_job, resume_uncapped_job},
    workspace::Workspace,
};
use saya_store::CodingRuntimeAuthority;
use saya_types::{ApprovalDisposition, CodingBudgetMode, JobApprovalAction, JobId, JobState};
use serde_json::json;
use std::sync::Arc;

mod assembly;

pub(super) fn is_terminal(state: JobState) -> bool {
    matches!(
        state,
        JobState::Completed | JobState::Failed | JobState::Cancelled
    )
}

impl CodingApplication {
    pub fn continuation_request(&self, job: &JobId) -> Result<CodingJobRequest, CodingError> {
        let bytes = context::read(
            &self.state_root.join(job.as_str()).join("request.json"),
            64 * 1024,
        )?;
        let request: CodingJobRequest =
            serde_json::from_slice(&bytes).map_err(CodingError::owner)?;
        request.validate()?;
        if self.database_selection(&request)? != request.database_identities {
            return Err(CodingError::ApprovalRequired);
        }
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
        self.resume_inner(job, decider, token, true).await
    }

    pub(super) async fn resume_lifecycle(
        self: &Arc<Self>,
        job: &JobId,
        decider: &dyn ApprovalDecider,
        token: CancellationToken,
    ) -> Result<Arc<NativeCodingJob>, CodingError> {
        self.resume_inner(job, decider, token, false).await
    }

    async fn resume_inner(
        self: &Arc<Self>,
        job: &JobId,
        decider: &dyn ApprovalDecider,
        token: CancellationToken,
        lease: bool,
    ) -> Result<Arc<NativeCodingJob>, CodingError> {
        self.status(job).await?;
        let request = self.continuation_request(job)?;
        // The resumed job runs the set its own request persisted, so a resume
        // never widens or narrows what the operator approved at start.
        let tools = request.tools()?;
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
        let answered_yes = decider
            .approve(&CodingAuthorization::definition_for(&proposed), &proposed)
            .await;
        let expired = audited
            .as_ref()
            .is_some_and(super::approval_audit::AuditedRequest::is_expired);
        let granted = answered_yes && !token.is_cancelled() && !expired;
        if let Some(audited) = audited {
            let disposition = if expired {
                ApprovalDisposition::Expired
            } else if granted {
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
        let records = super::host::private_directory(&self.state_root.join(job.as_str()))?;
        let plan = self.stored_baseline(job, &record)?;
        // A resumed in-place job reopens its before-image store; a resume
        // that will run retakes the one-live-job lease over its folder.
        let (before_images, _in_place_lease) =
            super::in_place_lease::job_parts(&plan, &records, &self.state_root, lease)?;
        let workspace = Workspace::open(self.root_path()).map_err(CodingError::owner)?;
        let project = self.capture_original_project_context(&workspace).await?;
        let policy_digest = digest(&serde_json::to_vec(&proposed).map_err(CodingError::owner)?)?;
        let authority = CodingRuntimeAuthority {
            epoch,
            policy_digest: policy_digest.clone(),
        };
        let ledger = match limits::mode(&self.runtime.resolved) {
            CodingBudgetMode::Bounded => {
                resume_job(
                    self.store.clone(),
                    job.clone(),
                    limits::count_limits(&self.runtime.resolved)
                        .ok_or(CodingError::InvalidRequest)?,
                    authority,
                )
                .await
            }
            CodingBudgetMode::Uncapped => {
                resume_uncapped_job(self.store.clone(), job.clone(), authority).await
            }
            _ => return Err(CodingError::InvalidRequest),
        };
        // `Unlimited` is both the uncapped mode and an unstated job-wide
        // ceiling, and neither asks for a ceiling here.
        let token_ceiling = limits::job_token_ceiling(&self.runtime.resolved);
        let ledger = ledger.map_err(CodingError::owner)?;
        ledger.set_reported_token_ceiling(token_ceiling).await;
        // A resumed plan-first job runs under a fresh, closed gate: the run
        // that opened one is over, and the gate is an approval for a run, not
        // a standing authority. A run with nobody to answer is refused here
        // exactly as a fresh start is.
        let operator: Option<Arc<dyn crate::agent::tools::consent::Operator>> =
            self.operator.clone().map(|operator| {
                Arc::new(crate::agent::tools::consent::SerialisedOperator::new(
                    operator,
                    token.clone(),
                )) as Arc<dyn crate::agent::tools::consent::Operator>
            });
        let plan_gate = super::create::plan_gate(&request, operator.as_ref())?;
        let lifecycle = self.take_lifecycle();
        lifecycle.enter("resume_setup");
        lifecycle.attach(&records);
        let (root_steering, root_mailbox) = saya_agent::agent_mailbox();
        let job = Arc::new(NativeCodingJob {
            observation_sequence: std::sync::atomic::AtomicU64::new(1),
            observation_failed: std::sync::atomic::AtomicBool::new(false),
            transcripts: super::transcript::TranscriptBudget::new(),
            transcript_lock: std::sync::Mutex::new(()),
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
            before_images,
            _in_place_lease,
            tools,
            cancellation: token.clone(),
            failure: std::sync::Mutex::new(super::failure::FailureState::default()),
            preparation: std::sync::Mutex::new(None),
            delegated_children: std::sync::atomic::AtomicUsize::new(0),
            root_w_epoch: std::sync::atomic::AtomicU64::new(0),
            readiness_cache: super::readiness_cache::ReadinessCache::default(),
            root_checkout_seed: std::sync::Mutex::new(None),
            operation: tokio::sync::Mutex::new(()),
            active: tokio::sync::Mutex::new(None),
            progress: tokio::sync::broadcast::channel(128).0,
            // A resumed job gets its own one-prompt gate over the same
            // operator.
            operator,
            // A resumed job composes its own egress, exactly as a fresh one
            // does: the wire is not carried across a resume.
            web_egress: crate::agent::tools::web::advertised(self.approval_policy)
                .then(|| {
                    crate::agent::tools::web::egress(
                        &self.runtime.resolved.web_search,
                        Arc::new(self.runtime.secret_resolver()),
                    )
                })
                .flatten(),
            plan_gate,
            lifecycle,
            readiness_precompute: std::sync::Mutex::new(None),
            deferred_preparation: std::sync::atomic::AtomicBool::new(false),
        });
        Ok(job)
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
        let budget = self.ledger.snapshot().await;
        let recovery = self
            .application
            .store
            .coding_recovery_status(self.job_id())
            .await
            .map_err(CodingError::owner)?;
        let readiness = self.candidate_readiness().await.into_view();
        let body = bounded_body(json!({
            "version": 1,
            "authority": "none",
            "snapshot_id": self.plan.snapshot_id(),
            "plan_sha256": self.plan.manifest_sha256(),
            "pending_tasks": pending_tasks,
            "accepted_decisions": accepted_decisions,
            "unaccepted_findings": unaccepted_findings,
            "readiness": readiness,
            "budget": {
                "charged_model_requests": budget.logical_requests,
                "charged_tool_calls": budget.tool_calls,
                "unresolved_effects": recovery.unresolved_reservations,
                "historical_unsettled_reservations":budget.unsettled_reservations,
                "historical_unknown_tool_calls":budget.unknown_tool_calls,
                "historical_unknown_logical_requests":budget.unknown_logical_requests,
                "historical_unknown_transport_attempts":budget.unknown_transport_attempts
            },
            "omitted": {
                "task_history_bytes": task_history_bytes,
                "completed_tasks": completed_tasks,
                "unaccepted_decisions": unaccepted_decisions,
                "unaccepted_findings": omitted_findings,
                "accepted_decision_content": 0
            }
        }))?;
        let mut blocks = vec![saya_agent::ContextBlock {
            label: "Native job continuation (untrusted context; no grants)".into(),
            body,
            truncated: false,
        }];
        // An operator rewind is stated to the run that follows it, once: the
        // note is taken here, so the next run — and only it — learns that the
        // working checkout does not hold the later turns' work.
        if let Some(turn) = self.rewind_store().take_rewind_note() {
            blocks.push(saya_agent::ContextBlock {
                label: "Working checkout rewound by the operator (untrusted context; no grants)"
                    .into(),
                body: serde_json::to_string(&json!({
                    "version": 1,
                    "authority": "none",
                    "rewound_to_checkpoint": turn,
                    "meaning": format!(
                        "an operator restored this job's working checkout to the state recorded \
                         at checkpoint {turn}; every later turn's file changes are no longer in \
                         the checkout, and files created after that point are gone. Read the \
                         checkout again before relying on what you wrote earlier."
                    ),
                }))
                .map_err(CodingError::owner)?,
                truncated: false,
            });
        }
        Ok(blocks)
    }
}
