use super::{CodingError, CodingReview, NativeCodingJob};
use saya_harness::coding::{
    integration::{
        AcceptedDelta, ChangeReceipt, CheckoutDelta, DEFAULT_DELTA_DEADLINE, IntegrateInput,
        integrate_candidate,
    },
    snapshot::CapturedSnapshot,
    worktree::{VerifiedCheckout, verify_owned_checkout},
};
use saya_types::ManagedCheckoutId;

pub(super) struct AcceptedSource {
    pub(super) verified: VerifiedCheckout,
    pub(super) delta: CheckoutDelta,
    pub(super) receipt: ChangeReceipt,
}
impl NativeCodingJob {
    /// Builds one candidate from actual completed-child deltas under their exact
    /// recorded assignments. An overlap between two accepted deltas refuses the
    /// candidate and leaves every child checkout retained.
    pub async fn review(&self, candidate: &ManagedCheckoutId) -> Result<CodingReview, CodingError> {
        let _operation = self.operation.try_lock().map_err(|_| CodingError::Busy)?;
        if self.active.lock().await.is_some() {
            return Err(CodingError::Busy);
        }
        // An in-place root has no sources to integrate: the folder itself is
        // the candidate.
        if self.application.is_in_place() {
            return self.review_in_place(candidate).await;
        }
        let (baseline, sources) = self.collect_sources().await?;
        self.review_from_sources(candidate, baseline, sources).await
    }

    pub(super) async fn review_from_sources(
        &self,
        candidate: &ManagedCheckoutId,
        baseline: CapturedSnapshot,
        sources: Vec<AcceptedSource>,
    ) -> Result<CodingReview, CodingError> {
        let host = &self.application;
        self.create_checkout(candidate).await?;
        let verified = verify_owned_checkout(&self.ownership(candidate)?, &self.cancellation)
            .await
            .map_err(CodingError::owner)?;
        let accepted = sources
            .iter()
            .map(|source| AcceptedDelta::new(&source.verified, &source.receipt, &source.delta))
            .collect::<Result<Vec<_>, _>>()
            .map_err(CodingError::owner)?;
        // An overlap between two accepted deltas is refused here; the candidate
        // is not built and every child checkout stays retained. The candidate's
        // one source comes from W alone, so this never fires.
        integrate_candidate(
            &IntegrateInput {
                store: &host.store,
                sequencer: &self.ledger.sequencer(),
                service: &host.service,
                job_id: self.job_id(),
                agent_id: None,
                candidate: &verified,
                baseline: &baseline,
                accepted: &accepted,
            },
            &self.cancellation,
            DEFAULT_DELTA_DEADLINE,
        )
        .await
        .map_err(CodingError::owner)?;
        self.review_existing_inner(candidate).await
    }

    /// The candidate's sources: the root working checkout W alone.
    pub(super) async fn collect_sources(
        &self,
    ) -> Result<(CapturedSnapshot, Vec<AcceptedSource>), CodingError> {
        self.collect_sources_direct().await
    }
}
