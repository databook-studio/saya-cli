//! Exact source/identity proposal shown to fresh host consent. Data is not a grant.
use super::{CodingApplication, CodingError, CodingJobRequest};
use saya_agent::CancellationToken;
use saya_harness::coding::local::JobBaseline;
use saya_harness::coding::snapshot::{
    SnapshotManifest, SnapshotOmissionPolicy, SnapshotOmissionReason, SnapshotSelection,
};
use saya_types::{JobId, SnapshotId};
use serde_json::{Value, json};
/// The start output names the sensitive paths the snapshot omits, capped so
/// the proposal stays under its 12 KiB bound: at most 64 paths and 4096 path
/// bytes, plus the total count.
const MAX_OMITTED_SENSITIVE_LISTED: usize = 64;
const MAX_OMITTED_SENSITIVE_BYTES: usize = 4096;

pub struct CodingApprovalRequest {
    pub(super) value: Value,
}
impl CodingApprovalRequest {
    pub fn request(&self) -> &Value {
        &self.value
    }
}
impl std::fmt::Debug for CodingApprovalRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodingApprovalRequest")
            .finish_non_exhaustive()
    }
}
impl CodingApplication {
    pub async fn propose(
        &self,
        request: &CodingJobRequest,
    ) -> Result<CodingApprovalRequest, CodingError> {
        Ok(self.propose_with_plan(request).await?.0)
    }

    /// The proposal and the exact baseline it approves. `create_with_plan`
    /// takes the same plan, so the `saya start` path inventories the working
    /// tree once instead of once per step. The plan is what the operator
    /// approved: capture binds its entries to the bytes it reads, and a
    /// changed working tree cannot add a path the plan did not state.
    pub async fn propose_with_plan(
        &self,
        request: &CodingJobRequest,
    ) -> Result<(CodingApprovalRequest, JobBaseline), CodingError> {
        self.propose_with_plan_until(request, &CancellationToken::new())
            .await
    }

    /// The proposal and plan under a caller's cancellation: a start that is
    /// cancelled stays cancelled rather than being retried as a fallback.
    pub async fn propose_with_plan_until(
        &self,
        request: &CodingJobRequest,
        token: &CancellationToken,
    ) -> Result<(CodingApprovalRequest, JobBaseline), CodingError> {
        self.lifecycle_enter("proposal");
        let plan = self.baseline_plan(request, token).await?;
        let proposal = self.proposal(request, &plan, 1, "start")?;
        Ok((proposal, plan))
    }
    /// The job's baseline plan: a stat-only walk for an in-place root, a Git
    /// snapshot plan for a repository. The walk is planned here and captured
    /// only after the operator approves it, so the proposal states exactly the
    /// files the baseline will hold.
    pub(super) async fn baseline_plan(
        &self,
        request: &CodingJobRequest,
        token: &CancellationToken,
    ) -> Result<JobBaseline, CodingError> {
        request.validate()?;
        let Some(local) = self.local_root() else {
            return Ok(JobBaseline::Git(Box::new(
                self.plan_request(request, token).await?,
            )));
        };
        let snapshot = SnapshotId::parse(request.job_id().as_str()).map_err(CodingError::owner)?;
        // An in-place baseline is an edit-time marker: the folder is stated,
        // never inventoried, so any tree size starts and reviews.
        let baseline = saya_harness::coding::local::LocalBaseline::edit_time(local, snapshot)
            .map_err(|error| CodingError::InPlaceBaseline {
                detail: error.to_string(),
            })?;
        Ok(JobBaseline::InPlace(baseline))
    }
    pub(super) async fn plan_request(
        &self,
        request: &CodingJobRequest,
        token: &CancellationToken,
    ) -> Result<SnapshotManifest, CodingError> {
        request.validate()?;
        let snapshot = SnapshotId::parse(request.job_id().as_str()).map_err(CodingError::owner)?;
        self.service
            .plan_snapshot(
                self.git_repository("snapshot plan")?,
                snapshot,
                SnapshotSelection::CurrentWorkingContent,
                SnapshotOmissionPolicy::new(self.git_repository("snapshot plan")?, vec![])
                    .map_err(CodingError::owner)?,
                token,
            )
            .await
            .map_err(CodingError::owner)
    }
    pub(super) fn proposal(
        &self,
        request: &CodingJobRequest,
        plan: &JobBaseline,
        epoch: u64,
        operation: &str,
    ) -> Result<CodingApprovalRequest, CodingError> {
        let root_path = self.root_path();
        let mut value = json!({"provider_bindings":self.provider_bindings()?,"operation":operation,"budget_mode":super::limits::mode(&self.runtime.resolved),"job_id":request.job_id,"task":request.task,"database_profiles":self.database_selection(request)?,
            "database_origin":if request.database_profiles.is_empty(){Value::Null}else{json!({"path":self.runtime.connections_path.as_ref().map(|path|saya_types::redact(&path.to_string_lossy())),"project_provided":self.runtime.connections_path.as_ref().is_some_and(|path|path.starts_with(root_path))})},
            "repository_id":plan.repository_id(),"repository_path":root_path,"snapshot_id":plan.snapshot_id(),"plan_sha256":plan.manifest_sha256(),"revocation_epoch":epoch});
        // In-place is stated only where it applies: a Git proposal keeps the
        // exact bytes (and therefore the policy digest) it always had.
        if let Some(local) = self.local_root() {
            let baseline = plan.in_place().ok_or(CodingError::InvalidRequest)?;
            value["mode"] = json!("in_place");
            value["root"] = json!({
                "path": local.path().to_string_lossy(),
                "root_id": local.id().as_str(),
                // The proposal precedes the start read, so it states the
                // conservative answer; the job record carries the real one.
                "git": false,
            });
            value["write_scope"] = json!(request.write_paths);
            value["baseline"] = json!({
                "kind": "edit_time",
                "plan_sha256": baseline.manifest_sha256(),
            });
            value["scratch"] = json!(".saya/inplace");
            value["no_git_history"] = json!(true);
            value["undo"] = json!({
                "checkpoints": super::rewind::MAX_REWIND_CHECKPOINTS,
                "baseline_restore": true,
                "commands": false,
                "downloads": false,
                "database_effects": false,
            });
            value["shell_reach"] =
                json!("the whole folder (sandbox root); host shell under ask/bypass");
        }
        // A Git snapshot omits untracked credential-shaped paths. The
        // operator's consent covers that omission, so the start output names
        // the paths (bounded) and the total count; an in-place job has no
        // snapshot omission and keeps its exact key set.
        if let Some(manifest) = plan.git() {
            let paths: Vec<&str> = manifest
                .omissions()
                .iter()
                .filter(|fact| fact.reason() == SnapshotOmissionReason::SensitiveUntracked)
                .map(|fact| fact.path().as_str())
                .collect();
            if !paths.is_empty() {
                let mut listed = Vec::new();
                let mut bytes = 0usize;
                for path in &paths {
                    if listed.len() == MAX_OMITTED_SENSITIVE_LISTED
                        || bytes.saturating_add(path.len()) > MAX_OMITTED_SENSITIVE_BYTES
                    {
                        break;
                    }
                    bytes += path.len();
                    listed.push(*path);
                }
                value["omitted_sensitive"] = json!(listed);
                value["omitted_sensitive_count"] = json!(paths.len());
            }
        }
        // The job's tool surface is part of what the operator approves, so a
        // narrowed set is stated here. A job that states none omits the key
        // entirely: its proposal — and the `policy_digest` derived from these
        // exact bytes — stays byte-identical to what it was before the setting
        // existed.
        if let Some(stated) = request.tools()?.stated() {
            value["tools_allow"] = json!(stated);
        }
        let proposal = CodingApprovalRequest { value };
        if proposal.value.to_string().len() > 12 * 1024 {
            return Err(CodingError::InvalidRequest);
        }
        Ok(proposal)
    }
    /// Reads the job's fixed baseline: `baseline.json` for an in-place job,
    /// `plan.json` for a Git job. The stored shape decides the variant and the
    /// captured snapshot's plan digest authenticates the bytes, so a job never
    /// resumes from a re-captured baseline.
    pub(super) fn stored_baseline(
        &self,
        job: &JobId,
        record: &saya_store::CodingJobRecord,
    ) -> Result<JobBaseline, CodingError> {
        let name = if self.is_in_place() {
            "baseline.json"
        } else {
            "plan.json"
        };
        let bytes = super::context::read(
            &self.state_root.join(job.as_str()).join(name),
            saya_harness::coding::snapshot::MAX_SNAPSHOT_MANIFEST_BYTES,
        )?;
        let captured = self
            .artifacts
            .read_snapshot(&record.snapshot_id)
            .map_err(CodingError::owner)?;
        let plan = JobBaseline::from_verified_bytes(&bytes, captured.plan_manifest_sha256())
            .map_err(CodingError::owner)?;
        if plan.snapshot_id() != &record.snapshot_id
            || plan.repository_id() != &record.repository_id
        {
            return Err(CodingError::NotOwned);
        }
        if self.is_in_place() {
            let root = super::root_checkout::InPlaceRootRecord::read(
                &self.state_root.join(job.as_str()).join("inplace-root.json"),
            )?;
            if root.root_id() != plan.repository_id() || root.snapshot_id() != plan.snapshot_id() {
                return Err(CodingError::NotOwned);
            }
        }
        Ok(plan)
    }
    pub async fn propose_resume(&self, job: &JobId) -> Result<CodingApprovalRequest, CodingError> {
        self.status(job).await?;
        let request = self.continuation_request(job)?;
        let record = self
            .store
            .get_coding_job(job)
            .await
            .map_err(CodingError::owner)?
            .ok_or(CodingError::NotOwned)?;
        let plan = self.stored_baseline(job, &record)?;
        let status = saya_harness::coding::coordinator::inspect_recovery(&self.store, job)
            .await
            .map_err(CodingError::owner)?;
        let epoch = status
            .authority
            .ok_or(CodingError::NotOwned)?
            .epoch
            .checked_add(1)
            .ok_or(CodingError::InvalidRequest)?;
        self.proposal(&request, &plan, epoch, "resume")
    }
}
