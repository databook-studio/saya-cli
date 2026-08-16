//! Request-scoped log of the candidate claims *persisted* this turn — the data
//! the runtime turns into one [`saya_agent::AgentEvent::KnowledgeProposed`] per
//! claim after the agent loop (spec P2d).
//!
//! Collects only; nothing here is persisted, and nothing reaches a provider.
//! The log records a [`ProposedClaimDto`] exactly when the store accepts a
//! proposal (the `Stored` arm in [`super::execute_contract_propose`]); a
//! refused, duplicate, or validation-failed proposal records nothing — so what
//! the runtime drains and emits is, by construction, only what was written.
//!
//! Mirrors the [`super::super::ObservationLog`] shape: a `Mutex<Vec<…>>` the
//! `&self` executor appends to and the owning runtime drains once. The cap is
//! the same per-turn proposal bound the tool enforces (`MAX_CANDIDATES_PER_TURN`
//! = 8); the tool refuses the ninth, so this is defense-in-depth, not the valve.

use saya_agent::ProposedClaimDto;
use std::sync::Mutex;

/// A request-scoped log of the candidate claims persisted this turn.
pub(crate) struct ProposedClaimsLog {
    claims: Mutex<Vec<ProposedClaimDto>>,
}

impl ProposedClaimsLog {
    pub(crate) fn new() -> Self {
        Self {
            claims: Mutex::new(Vec::new()),
        }
    }

    /// Appends a persisted claim. The tool's per-turn bound already caps this at
    /// eight; the cap here is defense-in-depth so a future caller cannot grow it.
    pub(crate) fn record(&self, claim: ProposedClaimDto) {
        let mut guard = self
            .claims
            .lock()
            .expect("proposed-claims log not poisoned");
        if guard.len() < super::MAX_CANDIDATES_PER_TURN {
            guard.push(claim);
        }
    }

    /// Returns and clears the turn's persisted claims, in the order they were
    /// stored. A second call returns nothing — a turn cannot double-emit.
    pub(crate) fn drain(&self) -> Vec<ProposedClaimDto> {
        std::mem::take(
            &mut *self
                .claims
                .lock()
                .expect("proposed-claims log not poisoned"),
        )
    }
}

impl Default for ProposedClaimsLog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use saya_types::{ClaimId, ClaimStatus};

    fn dto(profile: &str, object: &str, id: &str) -> ProposedClaimDto {
        ProposedClaimDto {
            claim_id: ClaimId::parse(id).unwrap(),
            profile: profile.into(),
            object: object.into(),
            kind: "table_alias".into(),
            value: "orders".into(),
            column: None,
            status: ClaimStatus::Candidate,
        }
    }

    #[test]
    fn drain_returns_claims_in_storage_order_and_clears() {
        let log = ProposedClaimsLog::new();
        log.record(dto("primary", "catalog.public.a", "c-1"));
        log.record(dto("primary", "catalog.public.b", "c-2"));
        let drained = log.drain();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].object, "catalog.public.a");
        assert_eq!(drained[1].object, "catalog.public.b");
        // A second drain is empty — a turn cannot double-emit.
        assert!(log.drain().is_empty());
    }

    /// The log never grows past the per-turn proposal bound, even if a future
    /// caller tried to over-record.
    #[test]
    fn the_log_caps_at_the_per_turn_bound() {
        let log = ProposedClaimsLog::new();
        for i in 0..(super::super::MAX_CANDIDATES_PER_TURN + 5) {
            log.record(dto("primary", "catalog.public.t", &format!("c-{i}")));
        }
        assert_eq!(
            log.drain().len(),
            super::super::MAX_CANDIDATES_PER_TURN,
            "the log honors the per-turn bound"
        );
    }
}
