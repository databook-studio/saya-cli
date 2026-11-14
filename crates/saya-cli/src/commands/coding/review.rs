use super::{CodingError, NativeCodingJob};
use saya_harness::coding::{
    integration::{
        AcceptedDelta, ApplyBackBounds, ApplyBackInput, AssignedPaths, CheckoutDelta,
        DEFAULT_DELTA_DEADLINE, EvidenceBinding, PlanIndex, apply_back, compute_delta,
        evaluate_receipt,
    },
    worktree::verify_admitted_checkout,
};
use saya_types::{JobId, ManagedCheckoutId, Sha256Digest};
use serde_json::{Value, json};
use std::path::Path;

pub struct CodingReview {
    pub(super) job: JobId,
    pub(super) epoch: u64,
    pub(super) binding: EvidenceBinding,
    pub(super) preview: Value,
}

#[cfg(all(test, unix))]
#[path = "review_tests.rs"]
mod tests;

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
        let _operation = self.operation.try_lock().map_err(|_| CodingError::Busy)?;
        if self.active.lock().await.is_some() {
            return Err(CodingError::Busy);
        }
        // An in-place root is the user's own folder: review reads it directly,
        // with no managed checkout to verify.
        if self.application.is_in_place() {
            return self.review_in_place(checkout).await;
        }
        self.review_existing_inner(checkout).await
    }
    pub(super) async fn review_existing_inner(
        &self,
        checkout: &ManagedCheckoutId,
    ) -> Result<CodingReview, CodingError> {
        let ownership = self.ownership(checkout)?;
        let verified = verify_admitted_checkout(&ownership, &self.cancellation)
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
        let ignored = self
            .application
            .service
            .ignored_paths(
                Path::new(verified.path().as_str()),
                &self.cancellation,
                DEFAULT_DELTA_DEADLINE,
            )
            .await
            .map_err(CodingError::owner)?;
        let delta = compute_delta(
            &verified,
            &baseline,
            &ignored,
            &self.cancellation,
            DEFAULT_DELTA_DEADLINE,
        )
        .map_err(CodingError::owner)?;
        let binding = EvidenceBinding::bind(
            &verified,
            &ignored,
            &self.cancellation,
            DEFAULT_DELTA_DEADLINE,
        )
        .map_err(CodingError::owner)?;
        let changes = delta
            .entries()
            .iter()
            .map(|entry| super::RootChange {
                path: entry.path().to_owned(),
                kind: entry.kind(),
                content_sha256: entry.content_sha256().cloned(),
                bytes: entry.bytes(),
            })
            .collect::<Vec<_>>();
        let diff = self.review_diff(
            &verified,
            super::review_diff::BaselineSource::Snapshot(&baseline),
            &changes,
        )?;
        let mut preview = json!({"candidate":checkout,"candidate_path":verified.path().as_str(),"source_sha256":binding.source_sha256(),"changes":delta.entries().iter().map(|entry|json!({"path":entry.path(),"kind":format!("{:?}",entry.kind()),"bytes":entry.bytes()})).collect::<Vec<_>>(),"diff":diff,"apply_requires_fresh_approval":true});
        add_delta_facts(&mut preview, &delta);
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
            preview,
        })
    }
    pub async fn apply_review(
        &self,
        review: &CodingReview,
        approved: bool,
    ) -> Result<Value, CodingError> {
        let _operation = self.operation.try_lock().map_err(|_| CodingError::Busy)?;
        if self.active.lock().await.is_some() {
            return Err(CodingError::Busy);
        }
        if !approved {
            return Err(CodingError::ApprovalRequired);
        }
        // An in-place apply finalizes: it verifies the review and records the
        // acknowledgement, never copying into the user's own folder.
        if self.application.is_in_place() {
            return self.apply_in_place(review).await;
        }
        if review.job != *self.job_id() || review.epoch != self.epoch {
            return Err(CodingError::StaleReview);
        }
        let ownership = self.ownership(review.binding.candidate_checkout_id())?;
        let verified = verify_admitted_checkout(&ownership, &self.cancellation)
            .await
            .map_err(CodingError::owner)?;
        let ignored = self
            .application
            .service
            .ignored_paths(
                Path::new(verified.path().as_str()),
                &self.cancellation,
                DEFAULT_DELTA_DEADLINE,
            )
            .await
            .map_err(CodingError::owner)?;
        review
            .binding
            .verify_at_use(
                Path::new(verified.path().as_str()),
                &ignored,
                &self.cancellation,
                DEFAULT_DELTA_DEADLINE,
            )
            .map_err(|_| CodingError::StaleReview)?;
        let baseline = self
            .application
            .artifacts
            .read_snapshot(verified.snapshot_id())
            .map_err(CodingError::owner)?;
        let delta = compute_delta(
            &verified,
            &baseline,
            &ignored,
            &self.cancellation,
            DEFAULT_DELTA_DEADLINE,
        )
        .map_err(CodingError::owner)?;
        // There is no job-level write-path ceiling: the root may change any
        // file in the checkout, so the candidate's whole content is in scope and
        // the operator's review and apply is the check. The receipt still binds
        // the delta to the checkout and keeps its byte bound.
        let receipt = evaluate_receipt(
            &verified,
            &delta,
            &AssignedPaths::whole_checkout(),
            64 * 1024 * 1024,
        );
        let accepted =
            [AcceptedDelta::new(&verified, &receipt, &delta).map_err(CodingError::owner)?];
        let git_plan = self.plan.git().ok_or(CodingError::InvalidRequest)?;
        let plan = PlanIndex::parse(git_plan.manifest_sha256(), git_plan.canonical_bytes())
            .map_err(CodingError::owner)?;
        let host = &self.application;
        apply_back(
            &ApplyBackInput {
                store: &host.store,
                sequencer: &self.ledger.sequencer(),
                job_id: self.job_id(),
                agent_id: None,
                repository: host.git_repository("apply")?,
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

/// The review facts that keep a partial delta honest: what was included, what
/// was not, and whether the delta covers the whole checkout.
pub(super) fn add_delta_facts(preview: &mut Value, delta: &CheckoutDelta) {
    preview["changes_truncated"] = json!(delta.changes_truncated());
    preview["included_files"] = json!(delta.len());
    preview["included_bytes"] = json!(delta.total_bytes());
    preview["not_included"] = json!(
        delta
            .not_included()
            .iter()
            .map(|item| json!({"path": item.path(), "reason": item.reason()}))
            .collect::<Vec<_>>()
    );
    preview["not_included_overflow"] = json!(delta.not_included_overflow());
}
