use super::{CodingError, NativeCodingJob, conflict_record::parse_retained_record};
use saya_harness::coding::{
    integration::{
        AcceptedDelta, ConflictDecision, ConflictQueue, ConflictResolution, DEFAULT_DELTA_DEADLINE,
        EvidenceBinding, IntegrateInput, detect_overlaps, integrate_candidate,
    },
    worktree::verify_owned_checkout,
};
use saya_types::{ManagedCheckoutId, Sha256Digest};

impl NativeCodingJob {
    pub(super) async fn resolve_conflict(
        &self,
        candidate: &ManagedCheckoutId,
        expected: &Sha256Digest,
        decision: ConflictDecision,
        approved: bool,
    ) -> Result<super::CodingReview, CodingError> {
        if !approved {
            return Err(CodingError::ApprovalRequired);
        }
        if self.active.lock().await.is_some() {
            return Err(CodingError::Busy);
        }
        let quiescence = self
            .ledger
            .try_quiescent()
            .await
            .map_err(CodingError::owner)?;
        if quiescence.job_id() != self.job_id() {
            return Err(CodingError::NotOwned);
        }
        let inspected = self.application.conflicts(self.job_id(), candidate).await?;
        let passive = &inspected["conflict"];
        if inspected["status"] != "retained"
            || inspected["provenance"]["complete"] != true
            || inspected["provenance"]["record_sha256"].as_str() != Some(expected.as_str())
        {
            return Err(CodingError::StaleReview);
        }
        let retained = parse_retained_record(
            &serde_json::to_vec(passive).map_err(CodingError::owner)?,
            self.job_id(),
        )?;
        let (baseline, sources) = self.collect_sources().await?;
        let accepted = sources
            .iter()
            .map(|source| {
                AcceptedDelta::new(&source.verified, &source.receipt, &source.delta)
                    .map_err(CodingError::owner)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let overlaps = detect_overlaps(&accepted)
            .err()
            .ok_or(CodingError::StaleReview)?;
        let mut queue = ConflictQueue::new();
        queue
            .record(candidate, &accepted, &overlaps)
            .map_err(CodingError::owner)?;
        let fresh = queue.open_for(candidate).ok_or(CodingError::StaleReview)?;
        let candidate_verified =
            verify_owned_checkout(&self.ownership(candidate), &self.cancellation)
                .await
                .map_err(CodingError::owner)?;
        let candidate_binding = EvidenceBinding::bind(
            &candidate_verified,
            &self.cancellation,
            DEFAULT_DELTA_DEADLINE,
        )
        .map_err(CodingError::owner)?;
        let source_bindings = sources
            .iter()
            .map(|source| source.binding.clone())
            .collect::<Vec<_>>();
        let fresh_bytes = super::conflicts::retained_bytes(
            self.job_id(),
            fresh,
            &candidate_binding,
            &source_bindings,
        )?;
        super::conflicts::validate_retained_bytes(&fresh_bytes, self.job_id())?;
        let fresh_record = parse_retained_record(&fresh_bytes, self.job_id())?;
        if fresh_record.record_sha256.as_deref() != Some(expected.as_str())
            || retained.candidate_source_sha256 != fresh_record.candidate_source_sha256
        {
            return Err(CodingError::StaleReview);
        }
        let kept = match queue
            .resolve(candidate, decision.clone())
            .map_err(CodingError::owner)?
        {
            ConflictResolution::KeptIndexes(indexes) => indexes,
            ConflictResolution::ManualEdit => return Err(CodingError::StaleReview),
            _ => return Err(CodingError::StaleReview),
        };
        let name = match decision {
            ConflictDecision::KeepFirst => "first",
            ConflictDecision::KeepSecond => "second",
            ConflictDecision::ManualEdit => return Err(CodingError::StaleReview),
            _ => return Err(CodingError::StaleReview),
        };
        let resolved =
            ManagedCheckoutId::parse(&format!("resolved-{name}-{}", &expected.as_str()[..32]))
                .map_err(CodingError::owner)?;
        let checkouts = self
            .application
            .store
            .list_managed_checkouts(self.job_id(), None, 100)
            .await
            .map_err(CodingError::owner)?;
        if checkouts.next_cursor.is_some()
            || checkouts
                .items
                .iter()
                .any(|record| record.ownership.managed_checkout_id() == &resolved)
        {
            return Err(CodingError::StaleReview);
        }
        self.create_checkout(&resolved).await?;
        let candidate = verify_owned_checkout(&self.ownership(&resolved), &self.cancellation)
            .await
            .map_err(CodingError::owner)?;
        let mut accepted = accepted.into_iter().map(Some).collect::<Vec<_>>();
        let selected = kept
            .iter()
            .map(|index| {
                accepted
                    .get_mut(*index)
                    .and_then(Option::take)
                    .ok_or(CodingError::InvalidRequest)
            })
            .collect::<Result<Vec<_>, _>>()?;
        integrate_candidate(
            &IntegrateInput {
                store: &self.application.store,
                sequencer: &self.ledger.sequencer(),
                job_id: self.job_id(),
                agent_id: None,
                candidate: &candidate,
                baseline: &baseline,
                accepted: &selected,
            },
            &self.cancellation,
            DEFAULT_DELTA_DEADLINE,
        )
        .await
        .map_err(CodingError::owner)?;
        drop(quiescence);
        self.review_existing(&resolved).await
    }
}
