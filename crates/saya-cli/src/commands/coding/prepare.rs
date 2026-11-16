//! Durable, content-free replay receipts for assembled review candidates.
use super::{
    CodingError, NativeCodingJob, context,
    prepare_receipt::{MAX_PREPARED_RECEIPT_BYTES, PreparedReceipt},
};
use saya_harness::coding::{
    integration::{DEFAULT_DELTA_DEADLINE, EvidenceBinding},
    worktree::verify_admitted_checkout,
};
use saya_types::{ManagedCheckoutId, Sha256Digest};
use serde_json::Value;

pub(super) struct SourceGeneration {
    pub(super) digest: Sha256Digest,
    pub(super) candidate: ManagedCheckoutId,
}

#[cfg(all(test, unix))]
#[path = "prepare_tests.rs"]
mod tests;

impl NativeCodingJob {
    /// Canonically prepares the current generation. A receipt is immutable per
    /// generation, so replay neither creates a checkout nor re-executes checks.
    pub(crate) async fn prepare_candidate(&self) -> Result<Value, CodingError> {
        let _operation = self.operation.try_lock().map_err(|_| CodingError::Busy)?;
        if self.active.lock().await.is_some() {
            return Err(CodingError::ChildrenActive);
        }
        let (baseline, sources) = self.collect_sources().await?;
        let generation = self.current_generation().await?;
        let path = self.prepared_receipt_path(&generation.digest);
        if path.exists() {
            return self.replay_prepared(&path, &generation).await;
        }
        if self
            .application
            .store
            .get_managed_checkout(self.job_id(), &generation.candidate)
            .await
            .map_err(CodingError::owner)?
            .is_some()
        {
            return Err(CodingError::PreparedCandidateState);
        }
        let review = self
            .review_from_sources(&generation.candidate, baseline, sources)
            .await?;
        let receipt = PreparedReceipt::from_review(
            generation.digest.clone(),
            generation.candidate.clone(),
            &review,
        )?;
        let verified =
            verify_admitted_checkout(&self.ownership(receipt.candidate())?, &self.cancellation)
                .await
                .map_err(|_| CodingError::PreparedCandidateState)?;
        let ignored = self.checkout_ignored_paths(&verified).await?;
        let binding = EvidenceBinding::bind(
            &verified,
            &ignored,
            &self.cancellation,
            DEFAULT_DELTA_DEADLINE,
        )
        .map_err(|_| CodingError::PreparedCandidateState)?;
        if binding.source_sha256() != receipt.reviewed_source_sha256() {
            return Err(CodingError::PreparedCandidateState);
        }
        self.validate_prepared_receipt(&receipt, std::path::Path::new(verified.path().as_str()))?;
        let bytes = serde_json::to_vec(&receipt).map_err(CodingError::owner)?;
        context::write_new(&path, &bytes, MAX_PREPARED_RECEIPT_BYTES)?;
        Ok(receipt.into_value())
    }

    pub(super) fn prepared_receipt_path(&self, generation: &Sha256Digest) -> std::path::PathBuf {
        self.records
            .join(format!("prepared-candidate-{}.json", generation.as_str()))
    }

    async fn replay_prepared(
        &self,
        path: &std::path::Path,
        generation: &SourceGeneration,
    ) -> Result<Value, CodingError> {
        let receipt = self.read_prepared_receipt(path, generation).await?;
        Ok(receipt.into_value())
    }

    pub(super) async fn read_prepared_receipt(
        &self,
        path: &std::path::Path,
        generation: &SourceGeneration,
    ) -> Result<PreparedReceipt, CodingError> {
        let receipt =
            PreparedReceipt::from_bytes(&context::read(path, MAX_PREPARED_RECEIPT_BYTES)?)?;
        if receipt.generation() != &generation.digest
            || receipt.candidate() != &generation.candidate
        {
            return Err(CodingError::PreparedCandidateState);
        }
        let verified =
            verify_admitted_checkout(&self.ownership(receipt.candidate())?, &self.cancellation)
                .await
                .map_err(|_| CodingError::PreparedCandidateState)?;
        let ignored = self.checkout_ignored_paths(&verified).await?;
        let binding = EvidenceBinding::bind(
            &verified,
            &ignored,
            &self.cancellation,
            DEFAULT_DELTA_DEADLINE,
        )
        .map_err(|_| CodingError::PreparedCandidateState)?;
        if binding.source_sha256() != receipt.reviewed_source_sha256() {
            return Err(CodingError::PreparedCandidateState);
        }
        self.validate_prepared_receipt(&receipt, std::path::Path::new(verified.path().as_str()))?;
        Ok(receipt)
    }

    fn validate_prepared_receipt(
        &self,
        receipt: &PreparedReceipt,
        _candidate_root: &std::path::Path,
    ) -> Result<(), CodingError> {
        receipt.validate_shape(self.job_id(), receipt.reviewed_source_sha256())?;
        Ok(())
    }
}
