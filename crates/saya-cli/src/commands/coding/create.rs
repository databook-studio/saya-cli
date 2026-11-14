use super::{
    CodingApplication, CodingAuthorization, CodingError, CodingJobRequest, NativeCodingJob,
    host::private_directory, limits,
};
use saya_agent::{ApprovalDecider, CancellationToken};
use saya_harness::{
    coding::{budget::JobBudgetLedger, local::JobBaseline, snapshot::MAX_SNAPSHOT_WARNINGS},
    workspace::Workspace,
};
use saya_store::NewCodingJob;
use saya_types::{
    ApprovalDisposition, CodingBudgetMode, CodingEventKind, CodingEventV1, JobApprovalAction,
    JobState, MAX_SNAPSHOT_WARNING_PATH_BYTES, Sha256Digest,
};
use sha2::{Digest, Sha256};
use std::{num::NonZeroU64, path::Path, sync::Arc};

impl CodingApplication {
    /// Captures the original project only after exact fresh approval; creates
    /// the durable job and budget owner once. Model text supplies no grants.
    pub async fn create(
        self: &Arc<Self>,
        request: CodingJobRequest,
        decider: &dyn ApprovalDecider,
        token: CancellationToken,
    ) -> Result<Arc<NativeCodingJob>, CodingError> {
        self.lifecycle_enter("proposal");
        let plan = self.baseline_plan(&request, &token).await?;
        self.build_job(request, plan, decider, token, false).await
    }

    /// The `saya start` entry over a plan the caller already approved, so the
    /// working tree is inventoried once instead of once per step. The plan is
    /// still the one the decider approves below and the one capture binds;
    /// nothing about it is trusted beyond that. A start with no operator has
    /// nobody waiting on a ready candidate, so it also defers the end-of-run
    /// preparation: `saya job apply` reviews and applies W directly.
    pub async fn create_for_start(
        self: &Arc<Self>,
        request: CodingJobRequest,
        plan: JobBaseline,
        decider: &dyn ApprovalDecider,
        token: CancellationToken,
    ) -> Result<Arc<NativeCodingJob>, CodingError> {
        let defer_preparation = self.operator.is_none();
        self.build_job(request, plan, decider, token, defer_preparation)
            .await
    }

    async fn build_job(
        self: &Arc<Self>,
        mut request: CodingJobRequest,
        plan: JobBaseline,
        decider: &dyn ApprovalDecider,
        token: CancellationToken,
        defer_preparation: bool,
    ) -> Result<Arc<NativeCodingJob>, CodingError> {
        request.validate()?;
        // Every warning becomes a durable event, so its path must satisfy the
        // event contract before the job row exists. A path that cannot be
        // recorded is refused here rather than after a half-created job.
        validate_snapshot_warning_paths(
            plan.warnings()
                .iter()
                .map(|warning| warning.path().as_str()),
        )?;
        let lifecycle = self.take_lifecycle();
        // The job's one operator gate, built before the job exists so the plan
        // gate below shares it: one prompt is pending at a time, whatever asks.
        let operator: Option<Arc<dyn crate::agent::tools::consent::Operator>> =
            self.operator.clone().map(|operator| {
                Arc::new(crate::agent::tools::consent::SerialisedOperator::new(
                    operator,
                    token.clone(),
                )) as Arc<dyn crate::agent::tools::consent::Operator>
            });
        let plan_gate = plan_gate(&request, operator.as_ref())?;
        // An in-place job writes the user's own folder, so a write tool with
        // no `--write` scope has nothing it may write: refuse it here rather
        // than compose a surface whose writes the boundary would deny.
        if self.is_in_place() && request.write_paths.is_empty() && request.states_write_tool()? {
            return Err(CodingError::InPlaceNeedsWriteScope);
        }
        let tools = request.tools()?;
        lifecycle.enter("approval");
        let proposed = self.proposal(&request, &plan, 1, "start")?.value;
        if !decider
            .approve(&CodingAuthorization::definition_for(&proposed), &proposed)
            .await
            || token.is_cancelled()
        {
            return Err(CodingError::ApprovalRequired);
        }
        request.database_identities = self.database_selection(&request)?;
        lifecycle.enter("context");
        let workspace = Workspace::open(self.root_path()).map_err(CodingError::owner)?;
        let project = self.capture_original_project_context(&workspace).await?;
        lifecycle.enter("snapshot");
        let captured = match &plan {
            saya_harness::coding::local::JobBaseline::Git(git_plan) => {
                let repository = self.git_repository("create")?;
                self.service
                    .capture_snapshot(repository, &workspace, git_plan, &self.artifacts, &token)
                    .await
            }
            saya_harness::coding::local::JobBaseline::InPlace(baseline) => {
                baseline.capture(&self.artifacts, &token).await
            }
        }
        .map_err(CodingError::owner)?;
        lifecycle.enter("job_setup");
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
        // Every non-fatal snapshot fact is a durable job event: the job
        // started with this content, and `job show` repeats the fact. The
        // warning names the path only, never its bytes.
        for (index, warning) in plan
            .warnings()
            .iter()
            .take(MAX_SNAPSHOT_WARNINGS)
            .enumerate()
        {
            let sequence = NonZeroU64::new(2 + index as u64).ok_or(CodingError::InvalidRequest)?;
            self.store
                .append_coding_event(
                    CodingEventV1::new(
                        sequence,
                        request.job_id.clone(),
                        None,
                        None,
                        CodingEventKind::SnapshotWarning {
                            path: warning.path().as_str().to_owned(),
                            reason: warning.reason(),
                        },
                    )
                    .map_err(CodingError::owner)?,
                )
                .await
                .map_err(CodingError::owner)?;
        }
        // The approval was given before the job existed, so its audit is
        // recorded now, as a granted request, before the ledger takes over the
        // event stream. A crash between the two events is cleaned at resume.
        if let Some(audited) = self
            .audit_request(&request.job_id, JobApprovalAction::Start, &proposed, 1)
            .await?
        {
            self.audit_close(&request.job_id, audited, ApprovalDisposition::Granted)
                .await?;
        }
        let policy_digest = digest(&serde_json::to_vec(&proposed).map_err(CodingError::owner)?)?;
        let ledger = match limits::mode(&self.runtime.resolved) {
            CodingBudgetMode::Bounded => {
                JobBudgetLedger::new(
                    self.store.clone(),
                    request.job_id.clone(),
                    limits::count_limits(&self.runtime.resolved)
                        .ok_or(CodingError::InvalidRequest)?,
                )
                .await
            }
            CodingBudgetMode::Uncapped => {
                JobBudgetLedger::new_uncapped(
                    self.store.clone(),
                    request.job_id.clone(),
                    self.runtime.resolved.agents.max_concurrent_model_requests as u64,
                    self.runtime.resolved.agents.max_children as u64,
                )
                .await
            }
            _ => return Err(CodingError::InvalidRequest),
        }
        .map_err(CodingError::owner)?;
        // `Unlimited` is both the uncapped mode and an unstated job-wide
        // ceiling, and neither asks for a ceiling here.
        ledger
            .set_reported_token_ceiling(limits::job_token_ceiling(&self.runtime.resolved))
            .await;
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
        match &plan {
            saya_harness::coding::local::JobBaseline::Git(_) => {
                super::context::write_new(
                    &records.join("plan.json"),
                    plan.canonical_bytes(),
                    saya_harness::coding::snapshot::MAX_SNAPSHOT_MANIFEST_BYTES,
                )?;
            }
            saya_harness::coding::local::JobBaseline::InPlace(baseline) => {
                super::context::write_new(
                    &records.join("baseline.json"),
                    plan.canonical_bytes(),
                    saya_harness::coding::snapshot::MAX_SNAPSHOT_MANIFEST_BYTES,
                )?;
                // Shell-made changes are this start read's difference; non-Git is unrecorded.
                let start = super::in_place_start::capture(
                    &self.service,
                    Path::new(baseline.root_path()),
                    &records,
                    &token,
                )
                .await?;
                let (git, start_head) = super::in_place_start::git_and_head(&start);
                super::root_checkout::InPlaceRootRecord::new(baseline, git, start_head)
                    .write(&records.join("inplace-root.json"))?;
            }
        }
        lifecycle.attach(&records);
        // An in-place job records edit-time before-images and holds the
        // one-live-job lease over its folder; a managed checkout needs
        // neither.
        let (before_images, _in_place_lease) =
            super::in_place_lease::job_parts(&plan, &records, &self.state_root, true)?;
        let (root_steering, root_mailbox) = saya_agent::agent_mailbox();
        Ok(Arc::new(NativeCodingJob {
            observation_sequence: std::sync::atomic::AtomicU64::new(1),
            observation_failed: std::sync::atomic::AtomicBool::new(false),
            transcripts: super::transcript::TranscriptBudget::new(),
            transcript_lock: std::sync::Mutex::new(()),
            root_steering,
            root_mailbox: tokio::sync::Mutex::new(Some(root_mailbox)),
            failure: std::sync::Mutex::new(super::failure::FailureState::default()),
            preparation: std::sync::Mutex::new(None),
            delegated_children: std::sync::atomic::AtomicUsize::new(0),
            root_w_epoch: std::sync::atomic::AtomicU64::new(0),
            readiness_cache: super::readiness_cache::ReadinessCache::default(),
            root_checkout_seed: std::sync::Mutex::new(None),
            operation: tokio::sync::Mutex::new(()),
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
            before_images,
            _in_place_lease,
            tools,
            cancellation: token.clone(),
            // One prompt pending at a time, whatever asks: the root's
            // host runs, its fetches, and every helper's are serialised through
            // this job's single gate.
            operator,
            // The job's one web egress, built only where a mode could reach
            // the network: `read-only` and `never` compose no wire at all. The
            // search backend rides along when the user configured one, and is
            // absent otherwise — the ordinary case.
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
            deferred_preparation: std::sync::atomic::AtomicBool::new(defer_preparation),
        }))
    }
}

/// Checks every warning path against the event contract's bound. The job row
/// and its Running event are written only after this succeeds.
pub(super) fn validate_snapshot_warning_paths<'a>(
    paths: impl IntoIterator<Item = &'a str>,
) -> Result<(), CodingError> {
    for path in paths {
        if path.is_empty() || path.len() > MAX_SNAPSHOT_WARNING_PATH_BYTES {
            return Err(CodingError::InvalidRequest);
        }
    }
    Ok(())
}

/// The plan gate a request asks for, or `None` for a job that never entered
/// plan mode. Plan mode needs someone to answer, something to open, and the one
/// tool that opens it: a run with no operator could never open the surface, a
/// job that states no `--write` path has none to open, and a `[tools] allow`
/// list that leaves `submit_plan` out would leave the posture with no way to
/// end — the model would simply write, plan mode having gated nothing. All
/// three are refused here, before the job exists, rather than silently ignored
/// or auto-approved.
pub(super) fn plan_gate(
    request: &CodingJobRequest,
    operator: Option<&Arc<dyn crate::agent::tools::consent::Operator>>,
) -> Result<Option<Arc<crate::agent::tools::coding::plan_gate::PlanGate>>, CodingError> {
    if !request.plan {
        return Ok(None);
    }
    let operator = operator.ok_or(CodingError::PlanNeedsOperator)?;
    if request.write_paths.is_empty() {
        return Err(CodingError::PlanNeedsWritePath);
    }
    if !crate::agent::tools::admits(
        &request.tools()?,
        crate::agent::tools::coding::plan_gate::SUBMIT_PLAN,
    ) {
        return Err(CodingError::PlanNeedsGateTool);
    }
    Ok(Some(Arc::new(
        crate::agent::tools::coding::plan_gate::PlanGate::new(
            Some(Arc::clone(operator)),
            crate::agent::tools::consent::Caller::Root {
                job_id: request.job_id.clone(),
            },
        ),
    )))
}

pub(super) fn digest(bytes: &[u8]) -> Result<Sha256Digest, CodingError> {
    let digest = Sha256::digest(bytes);
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    Sha256Digest::parse(&hex).map_err(CodingError::owner)
}

pub(super) fn actor_correlation_digest(parts: &[&str]) -> saya_agent::ActorCorrelationDigest {
    let mut hasher = Sha256::new();
    hasher.update(b"saya-coding-actor-v1");
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    saya_agent::ActorCorrelationDigest::from_bytes(hasher.finalize().into())
}

#[cfg(all(test, unix))]
mod actor_correlation_tests {
    use super::{actor_correlation_digest, validate_snapshot_warning_paths};
    use crate::commands::coding::CodingError;
    use saya_types::MAX_SNAPSHOT_WARNING_PATH_BYTES;

    /// The event contract's path bound is checked before the job row exists,
    /// so an over-long warning path can never fail a half-created job.
    #[test]
    fn warning_paths_over_the_event_bound_are_refused() {
        let over = "a".repeat(MAX_SNAPSHOT_WARNING_PATH_BYTES + 1);
        assert!(matches!(
            validate_snapshot_warning_paths([over.as_str()]),
            Err(CodingError::InvalidRequest)
        ));
        assert!(matches!(
            validate_snapshot_warning_paths([""]),
            Err(CodingError::InvalidRequest)
        ));
        let at_bound = "a".repeat(MAX_SNAPSHOT_WARNING_PATH_BYTES);
        assert!(validate_snapshot_warning_paths([at_bound.as_str()]).is_ok());
        assert!(validate_snapshot_warning_paths(["tests/certs/cert.pem"]).is_ok());
    }

    #[test]
    fn durable_actor_tuple_changes_are_distinct_and_repeatable() {
        let root = actor_correlation_digest(&["root", "job-1"]);
        assert_eq!(root, actor_correlation_digest(&["root", "job-1"]));
        assert_ne!(root, actor_correlation_digest(&["root", "job-2"]));

        let child = actor_correlation_digest(&["child", "job-1", "agent-1", "inv-1"]);
        assert_ne!(
            child,
            actor_correlation_digest(&["child", "job-1", "agent-1", "inv-2"])
        );
        assert_ne!(root, child);
        assert_eq!(child.as_bytes().len(), 32);
    }
}
