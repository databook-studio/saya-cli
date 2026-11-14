//! Candidate preparation: the one source is the root's own working checkout W,
//! with no children and no task records required — a job where the coordinator
//! worked alone is the normal case, not an error. There is no job-level
//! write-path ceiling: W's whole content is in scope and the operator's
//! review and apply is the check.
//!
//! The candidate is assembled by `review_from_sources` (the review, checks,
//! receipt and apply-back gates).

use super::integrate::AcceptedSource;
use super::prepare::SourceGeneration;
use super::{CodingError, NativeCodingJob, create::digest};
use saya_harness::coding::integration::{
    AssignedPaths, DEFAULT_DELTA_DEADLINE, EvidenceBinding, compute_delta, evaluate_receipt,
};
use saya_harness::coding::snapshot::CapturedSnapshot;
use saya_types::{ManagedCheckoutId, Sha256Digest, SnapshotId};
use serde_json::json;

/// Longest source delta a receipt covers: the byte bound the receipt keeps
/// with no path ceiling.
const MAX_SOURCE_DELTA_BYTES: u64 = 64 * 1024 * 1024;

impl NativeCodingJob {
    /// The current source generation: W alone, and the merge completions
    /// applied into it.
    pub(super) async fn current_generation(&self) -> Result<SourceGeneration, CodingError> {
        self.source_generation_direct().await
    }

    /// Builds the single `AcceptedSource` from the root working checkout W:
    /// verified (`Active | Retained`), delta vs the baseline, and receipt-bound
    /// to the checkout's whole content. A checkout with no changes prepares
    /// nothing and says so.
    pub(super) async fn collect_sources_direct(
        &self,
    ) -> Result<(CapturedSnapshot, Vec<AcceptedSource>), CodingError> {
        let baseline = self
            .application
            .artifacts
            .read_snapshot(self.plan.snapshot_id())
            .map_err(CodingError::owner)?;
        let Some(source) = self.working_source(&baseline).await? else {
            // An unchanged W still replays its current generation's receipt; a
            // generation with neither changes nor a receipt prepares nothing.
            let generation = self.current_generation().await?;
            if !self.prepared_receipt_path(&generation.digest).exists() {
                return Err(CodingError::NothingToPrepare);
            }
            return Ok((baseline, Vec::new()));
        };
        Ok((baseline, vec![source]))
    }

    /// The current source generation: a digest over the snapshot and W's
    /// source content. Its candidate id is `candidate-<digest>`.
    pub(super) async fn source_generation_direct(&self) -> Result<SourceGeneration, CodingError> {
        let verified = self
            .verified_root_checkout()
            .await?
            .ok_or(CodingError::NothingToPrepare)?;
        let ignored = self.checkout_ignored_paths(&verified).await?;
        self.source_generation_from(&verified, &ignored)
    }

    /// The generation of an already-verified W. The advisory readiness verifies
    /// W once for both the delta and the generation, so the binding is not paid
    /// twice.
    pub(super) fn source_generation_from(
        &self,
        verified: &saya_harness::coding::worktree::VerifiedCheckout,
        ignored: &saya_harness::coding::git::IgnoredPaths,
    ) -> Result<SourceGeneration, CodingError> {
        let binding = EvidenceBinding::bind(
            verified,
            ignored,
            &self.cancellation,
            DEFAULT_DELTA_DEADLINE,
        )
        .map_err(CodingError::owner)?;
        let digest = generation_digest(
            self.plan.snapshot_id(),
            self.plan.manifest_sha256(),
            binding.source_sha256(),
        )?;
        let candidate = ManagedCheckoutId::parse(&format!("candidate-{}", digest.as_str()))
            .map_err(CodingError::owner)?;
        Ok(SourceGeneration { digest, candidate })
    }

    /// One `AcceptedSource` built from W, or `None` when W is unchanged.
    async fn working_source(
        &self,
        baseline: &CapturedSnapshot,
    ) -> Result<Option<AcceptedSource>, CodingError> {
        let Some(verified) = self.verified_root_checkout().await? else {
            return Ok(None);
        };
        let ignored = self.checkout_ignored_paths(&verified).await?;
        let delta = compute_delta(
            &verified,
            baseline,
            &ignored,
            &self.cancellation,
            DEFAULT_DELTA_DEADLINE,
        )
        .map_err(CodingError::owner)?;
        if delta.is_empty() {
            return Ok(None);
        }
        // The whole checkout is the assignment: every change W holds is in
        // scope, so the receipt binds the delta to the checkout and keeps its
        // byte bound while the path list restricts nothing.
        let receipt = evaluate_receipt(
            &verified,
            &delta,
            &AssignedPaths::whole_checkout(),
            MAX_SOURCE_DELTA_BYTES,
        );
        Ok(Some(AcceptedSource {
            verified,
            delta,
            receipt,
        }))
    }
}

/// The generation digest. Every input that can change what the candidate holds
/// is in the preimage: the snapshot the job started from, a fixed domain tag,
/// and W's current source content.
fn generation_digest(
    snapshot_id: &SnapshotId,
    snapshot_sha256: &Sha256Digest,
    source_sha256: &Sha256Digest,
) -> Result<Sha256Digest, CodingError> {
    digest(
        &serde_json::to_vec(&json!({
            "snapshot_id": snapshot_id,
            "snapshot_sha256": snapshot_sha256,
            "mode": "direct",
            "source_sha256": source_sha256,
        }))
        .map_err(CodingError::owner)?,
    )
}

#[cfg(all(test, unix))]
#[path = "prepare_direct_tests.rs"]
mod tests;
