use super::{CodingError, NativeCodingJob, diagnostic_baseline::DiagnosticDeltaState};
use saya_harness::coding::{
    checks::{CheckDiagnostics, CheckReceipt},
    integration::{
        AcceptedDelta, ApplyBackBounds, ApplyBackInput, AssignedPaths, DEFAULT_DELTA_DEADLINE,
        EvidenceBinding, PlanIndex, apply_back, compute_delta, evaluate_receipt,
    },
    worktree::verify_owned_checkout,
};
use saya_types::{JobId, ManagedCheckoutId, Sha256Digest};
use serde_json::{Value, json};
use std::path::Path;

pub struct CodingReview {
    pub(super) job: JobId,
    pub(super) epoch: u64,
    pub(super) binding: EvidenceBinding,
    pub(super) checks: Vec<ReviewCheck>,
    pub(super) preview: Value,
}
pub(super) enum ReviewCheck {
    Plain(CheckReceipt),
    Diagnostics(CheckDiagnostics),
}
impl ReviewCheck {
    pub(super) fn receipt(&self) -> &CheckReceipt {
        match self {
            Self::Plain(receipt) => receipt,
            Self::Diagnostics(evidence) => evidence.receipt(),
        }
    }
    pub(super) fn diagnostics(&self) -> Option<&CheckDiagnostics> {
        match self {
            Self::Diagnostics(evidence) => Some(evidence),
            Self::Plain(_) => None,
        }
    }
    fn preview(&self, delta: Option<&DiagnosticDeltaState>) -> Value {
        let mut preview = json!({
            "profile":self.receipt().profile_id(),
            "outcome":self.receipt().outcome(),
            "process":super::checks::process_facts(self.receipt()),
            "successful":self.receipt().is_successful()
        });
        match self {
            Self::Plain(_) => {
                super::checks::merge_json(&mut preview, super::checks::diagnostic_facts(None, None))
            }
            Self::Diagnostics(evidence) => {
                super::checks::merge_json(
                    &mut preview,
                    super::checks::diagnostic_facts(
                        Some(evidence.format()),
                        Some(evidence.diagnostics()),
                    ),
                );
                if let Some(delta) = delta {
                    super::checks::merge_json(&mut preview, delta.preview());
                }
            }
        }
        preview
    }
}
impl CodingReview {
    pub fn source_sha256(&self) -> &Sha256Digest {
        self.binding.source_sha256()
    }
    pub fn preview(&self) -> &Value {
        &self.preview
    }
}
impl NativeCodingJob {
    pub async fn review_existing(
        &self,
        checkout: &ManagedCheckoutId,
    ) -> Result<CodingReview, CodingError> {
        let ownership = self.ownership(checkout);
        let verified = verify_owned_checkout(&ownership, &self.cancellation)
            .await
            .map_err(CodingError::owner)?;
        if verified.agent_id().is_some() {
            return Err(CodingError::NotOwned);
        }
        let baseline = self
            .application
            .artifacts
            .read_snapshot(verified.snapshot_id())
            .map_err(CodingError::owner)?;
        let delta = compute_delta(
            &verified,
            &baseline,
            &self.cancellation,
            DEFAULT_DELTA_DEADLINE,
        )
        .map_err(CodingError::owner)?;
        let binding = EvidenceBinding::bind(&verified, &self.cancellation, DEFAULT_DELTA_DEADLINE)
            .map_err(CodingError::owner)?;
        let checks = self.candidate_checks(checkout).await?;
        let diagnostic_deltas = self.prepare_diagnostic_deltas(checkout, &checks).await?;
        binding
            .verify_at_use(
                Path::new(verified.path().as_str()),
                &self.cancellation,
                DEFAULT_DELTA_DEADLINE,
            )
            .map_err(|_| CodingError::StaleReview)?;
        let diff = self.review_diff(&verified, &baseline, &delta)?;
        let diagnostics_configured = checks
            .iter()
            .any(|check| matches!(check, ReviewCheck::Diagnostics(_)));
        let preview = json!({"candidate":checkout,"candidate_path":verified.path().as_str(),"source_sha256":binding.source_sha256(),"changes":delta.entries().iter().map(|entry|json!({"path":entry.path(),"kind":format!("{:?}",entry.kind()),"bytes":entry.bytes()})).collect::<Vec<_>>(),"diff":diff,
            "checks":checks.iter().zip(diagnostic_deltas.iter()).map(|(check,delta)|check.preview(delta.as_ref())).collect::<Vec<_>>(),"checks_configured":!checks.is_empty(),"diagnostics_configured":diagnostics_configured,"diagnostic_delta":super::diagnostic_baseline::summary(&diagnostic_deltas),"apply_requires_fresh_approval":true});
        super::context::append_line(
            &self
                .records
                .join(format!("review-{}.jsonl", checkout.as_str())),
            &serde_json::to_vec(&preview).map_err(CodingError::owner)?,
            1024 * 1024,
        )?;
        Ok(CodingReview {
            job: self.job_id().clone(),
            epoch: self.epoch,
            binding,
            checks,
            preview,
        })
    }
    pub async fn apply_review(
        &self,
        review: &CodingReview,
        approved: bool,
    ) -> Result<Value, CodingError> {
        if !approved {
            return Err(CodingError::ApprovalRequired);
        }
        if review.job != *self.job_id() || review.epoch != self.epoch {
            return Err(CodingError::StaleReview);
        }
        let ownership = self.ownership(review.binding.candidate_checkout_id());
        let verified = verify_owned_checkout(&ownership, &self.cancellation)
            .await
            .map_err(CodingError::owner)?;
        review
            .binding
            .verify_at_use(
                Path::new(verified.path().as_str()),
                &self.cancellation,
                DEFAULT_DELTA_DEADLINE,
            )
            .map_err(|_| CodingError::StaleReview)?;
        if review.checks.is_empty()
            || review
                .checks
                .iter()
                .any(|check| !check.receipt().is_successful())
        {
            return Err(CodingError::ChecksRequired);
        }
        for check in &review.checks {
            check
                .receipt()
                .verify_at_use(&ownership, &self.cancellation)
                .await
                .map_err(|_| CodingError::StaleReview)?;
        }
        let baseline = self
            .application
            .artifacts
            .read_snapshot(verified.snapshot_id())
            .map_err(CodingError::owner)?;
        let delta = compute_delta(
            &verified,
            &baseline,
            &self.cancellation,
            DEFAULT_DELTA_DEADLINE,
        )
        .map_err(CodingError::owner)?;
        let assigned =
            AssignedPaths::parse(self.request.write_paths.clone()).map_err(CodingError::owner)?;
        let receipt = evaluate_receipt(&verified, &delta, &assigned, 64 * 1024 * 1024);
        let accepted =
            [AcceptedDelta::new(&verified, &receipt, &delta).map_err(CodingError::owner)?];
        let plan = PlanIndex::parse(self.plan.manifest_sha256(), self.plan.canonical_bytes())
            .map_err(CodingError::owner)?;
        let host = &self.application;
        apply_back(
            &ApplyBackInput {
                store: &host.store,
                sequencer: &self.ledger.sequencer(),
                job_id: self.job_id(),
                agent_id: None,
                repository: &host.repository,
                service: &host.service,
                candidate: &verified,
                baseline: &baseline,
                plan: &plan,
                accepted: &accepted,
                artifacts: &host.artifacts,
                state_dir: &host.state_root,
            },
            &self.cancellation,
            ApplyBackBounds::default(),
        )
        .await
        .map_err(CodingError::owner)?;
        Ok(json!({"applied":true,"source_sha256":review.source_sha256()}))
    }
}
