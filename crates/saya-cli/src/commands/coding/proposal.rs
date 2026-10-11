//! Exact source/identity proposal shown to fresh host consent. Data is not a grant.
use super::{CodingApplication, CodingError, CodingJobRequest};
use saya_agent::CancellationToken;
use saya_harness::coding::snapshot::{SnapshotManifest, SnapshotOmissionPolicy, SnapshotSelection};
use saya_types::{JobId, SnapshotId};
use serde_json::{Value, json};
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
        let plan = self
            .plan_request(request, &CancellationToken::new())
            .await?;
        self.proposal(request, &plan, 1, "start")
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
                &self.repository,
                snapshot,
                SnapshotSelection::CurrentWorkingContent,
                SnapshotOmissionPolicy::new(&self.repository, vec![])
                    .map_err(CodingError::owner)?,
                token,
            )
            .await
            .map_err(CodingError::owner)
    }
    pub(super) fn proposal(
        &self,
        request: &CodingJobRequest,
        plan: &SnapshotManifest,
        epoch: u64,
        operation: &str,
    ) -> Result<CodingApprovalRequest, CodingError> {
        let proposal = CodingApprovalRequest {
            value: json!({"provider_bindings":self.provider_bindings()?,"operation":operation,"job_id":request.job_id,"task":request.task,"write_paths":request.write_paths,"programs":request.programs,
            "repository_id":plan.repository_id(),"repository_path":self.repository.root(),"snapshot_id":plan.snapshot_id(),"plan_sha256":plan.manifest_sha256(),"revocation_epoch":epoch}),
        };
        if proposal.value.to_string().len() > 12 * 1024 {
            return Err(CodingError::InvalidRequest);
        }
        Ok(proposal)
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
        let captured = self
            .artifacts
            .read_snapshot(&record.snapshot_id)
            .map_err(CodingError::owner)?;
        let bytes = super::context::read(
            &self.state_root.join(job.as_str()).join("plan.json"),
            saya_harness::coding::snapshot::MAX_SNAPSHOT_MANIFEST_BYTES,
        )?;
        let plan = SnapshotManifest::from_verified_bytes(&bytes, captured.plan_manifest_sha256())
            .map_err(CodingError::owner)?;
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
