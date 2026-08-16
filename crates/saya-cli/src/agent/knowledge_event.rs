//! The recall→event mapping — spec P1b §3. Pure, so the three-state decision
//! (Off / Skipped / Ran) and the claim mapping are unit-testable without a live
//! database; the runtime calls [`knowledge_supplied_event`] and emits it.
//!
//! The payload says **supplied**, never *used*: a confirmed claim being
//! supplied does not mean the generated SQL honoured it (we have measured that
//! it frequently does not). The opaque [`saya_types::ProfileIdentity`] never
//! crosses — the receipt carries the human-facing name only, and the DTO has
//! no identity field, by construction.

use crate::contracts::{
    RecallMode, RecallOutcomeKind, RecallReceipt, SuppliedClaim, SuppliedContract,
};
use saya_agent::{AgentEvent, KnowledgeOutcome, SuppliedClaimDto, SuppliedContractDto};

/// Builds the single [`AgentEvent::KnowledgeSupplied`] for a turn from the
/// runtime's arm (which recall mode, the privacy gate) and the assembled
/// receipt (spec P1b §3).
///
/// `recall_mode == None` is [`KnowledgeOutcome::Off`] (recall disabled by
/// config). `Some` under a closed privacy gate is [`KnowledgeOutcome::Skipped`]
/// (SAYA was not allowed to look). Otherwise recall ran and `receipt` carries
/// what it supplied; the `Ran` arm maps its claims onto the event DTO. The
/// runtime is the only caller and passes `Some` here by construction; `None`
/// in the `Ran` arm is unreachable, so we fail soft to an empty `Ran` rather
/// than panic (recall never fails the turn, spec §3).
pub(crate) fn knowledge_supplied_event(
    recall_mode: Option<RecallMode>,
    allow_query_data: bool,
    receipt: Option<&RecallReceipt>,
) -> AgentEvent {
    match (recall_mode, allow_query_data) {
        // `recall = off`: SAYA did not look. Distinct from the privacy gate
        // (Skipped) — a user reads "configured off" differently from "not
        // allowed to look" (spec §3).
        (None, _) => AgentEvent::knowledge_supplied(KnowledgeOutcome::Off, Vec::new(), 0),
        // Privacy gate closed: SAYA was not allowed to look. No store query.
        (Some(_), false) => {
            AgentEvent::knowledge_supplied(KnowledgeOutcome::Skipped, Vec::new(), 0)
        }
        // Recall ran. Map the receipt's supplied claims onto the event DTO;
        // the identity never crosses (the receipt carries the name only).
        (Some(_), true) => match receipt {
            Some(receipt) => {
                let store_unavailable = matches!(
                    receipt.kind,
                    RecallOutcomeKind::Ran {
                        store_unavailable: true
                    }
                );
                AgentEvent::knowledge_supplied(
                    KnowledgeOutcome::Ran { store_unavailable },
                    receipt.supplied.iter().map(map_supplied_contract).collect(),
                    receipt.dropped_by_bounds,
                )
            }
            None => AgentEvent::knowledge_supplied(
                KnowledgeOutcome::Ran {
                    store_unavailable: false,
                },
                Vec::new(),
                0,
            ),
        },
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
