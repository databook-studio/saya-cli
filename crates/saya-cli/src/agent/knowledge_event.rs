//! The recall→event mapping is pure, so the three-state decision
//! (Off / Skipped / Ran) and the claim mapping are unit-testable without a live
//! database; the runtime calls [`knowledge_supplied_event`] and emits it.
//!
//! The receipt is the single source of truth for the outcome: whether recall
//! was configured off, skipped by the privacy gate, or ran against the store.
//!
//! The payload says **supplied**, never *used*: a confirmed claim being
//! supplied does not mean the generated SQL honoured it (we have measured that
//! it frequently does not). The opaque [`saya_types::ProfileIdentity`] never
//! crosses — the receipt carries the human-facing name only, and the DTO has
//! no identity field, by construction.

use crate::contracts::{RecallOutcomeKind, RecallReceipt, SuppliedClaim, SuppliedContract};
use saya_agent::{AgentEvent, KnowledgeOutcome, SuppliedClaimDto, SuppliedContractDto};

/// Builds the single [`AgentEvent::KnowledgeSupplied`] for a turn directly from
/// the assembled [`RecallReceipt`].
///
/// The receipt is the single source of truth for the outcome:
/// - [`RecallOutcomeKind::ConfiguredOff`] maps to [`KnowledgeOutcome::Off`] (recall disabled by config).
/// - [`RecallOutcomeKind::PrivacyGateClosed`] maps to [`KnowledgeOutcome::Skipped`] (SAYA was not permitted to look).
/// - [`RecallOutcomeKind::Ran`] maps to [`KnowledgeOutcome::Ran`] with the supplied claims and dropped bounds count from the receipt.
pub(crate) fn knowledge_supplied_event(receipt: &RecallReceipt) -> AgentEvent {
    match receipt.kind {
        RecallOutcomeKind::ConfiguredOff => {
            AgentEvent::knowledge_supplied(KnowledgeOutcome::Off, Vec::new(), 0)
        }
        RecallOutcomeKind::PrivacyGateClosed => {
            AgentEvent::knowledge_supplied(KnowledgeOutcome::Skipped, Vec::new(), 0)
        }
        RecallOutcomeKind::Ran { store_unavailable } => AgentEvent::knowledge_supplied(
            KnowledgeOutcome::Ran { store_unavailable },
            receipt.supplied.iter().map(map_supplied_contract).collect(),
            receipt.dropped_by_bounds,
        ),
    }
}

fn map_supplied_contract(contract: &SuppliedContract) -> SuppliedContractDto {
    SuppliedContractDto {
        profile: contract.profile.clone(),
        object: contract.object.clone(),
        schema_state: contract.schema_state.to_string(),
        claims: contract.claims.iter().map(map_supplied_claim).collect(),
    }
}

fn map_supplied_claim(claim: &SuppliedClaim) -> SuppliedClaimDto {
    SuppliedClaimDto {
        claim_id: claim.claim_id.clone(),
        kind: claim.kind.to_string(),
        value: claim.value.clone(),
        column: claim.column.clone(),
        status: claim.status,
    }
}
