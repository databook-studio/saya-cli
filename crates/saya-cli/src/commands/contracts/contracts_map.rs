//! Mapping the typed `crate::contracts::view` results to render-owned DTOs.
//!
//! Presentation only: turns a `RetrievedContract` (which carries the opaque
//! `ProfileIdentity` inside its `DatabaseObjectRef`) into a `ContractView`
//! (which carries the profile *name* and a qualified object string). The
//! identity is dropped here and must never appear in any field — the DTO has no
//! field for it, and the only name we render is the one the caller resolved.

use crate::contracts::{ContractConflict, ContractSchemaState, QueuedCandidate, RetrievedContract};
use crate::render::{ContractClaimView, ContractConflictView, ContractQueueItemView, ContractView};
use saya_store::StoredClaim;
use saya_types::ClaimPayload;

/// Maps one retrieved contract to its render DTO. `profile_name` is the name the
/// adapter resolved for this contract's profile — never the identity.
///
/// `pub(crate)` so the agent contract tools (2b-3a) reuse this same identity-dropping
/// mapping instead of carrying a second one that could leak the opaque identity.
pub(crate) fn contract_view(contract: &RetrievedContract, profile_name: &str) -> ContractView {
    ContractView {
        profile: profile_name.to_string(),
        object: contract.object.qualified_name(),
        schema_state: schema_state_str(contract.schema_state),
        claims: contract.claims.iter().map(claim_view).collect(),
        conflicts: contract.conflicts.iter().map(conflict_view).collect(),
        truncated: contract.truncated,
    }
}

fn schema_state_str(state: ContractSchemaState) -> String {
    match state {
        ContractSchemaState::Current => "current",
        ContractSchemaState::NeedsReview => "needs_review",
        ContractSchemaState::Stale => "stale",
        ContractSchemaState::LiveSchemaUnavailable => "live_schema_unavailable",
    }
    .into()
}

fn claim_view(claim: &StoredClaim) -> ContractClaimView {
    // A recallable claim always has a payload; a forgotten tombstone does not,
    // and recall/show filter to recallable claims, so `None` is a defensive
    // fallback rather than a path that should render.
    let (kind, value, column) = match claim.payload.as_ref() {
        Some(payload) => render_payload(payload),
        None => (String::new(), String::new(), None),
    };
    ContractClaimView {
        claim_id: claim.id.as_str().to_string(),
        kind,
        origin: claim.origin.as_str().to_string(),
        status: claim.status.as_str().to_string(),
        value,
        column,
    }
}

/// The short rendered form of a payload: the alias, the text, the role, or the
/// column — matching the render snapshots in `tests/contract_render.rs`.
pub(crate) fn render_payload(payload: &ClaimPayload) -> (String, String, Option<String>) {
    match payload {
        ClaimPayload::TableDescription { text, .. } => (payload.kind().into(), text.clone(), None),
        ClaimPayload::TableAlias { alias, .. } => (payload.kind().into(), alias.clone(), None),
        ClaimPayload::TableGrain { description, .. } => {
            (payload.kind().into(), description.clone(), None)
        }
        ClaimPayload::ColumnDescription { column, text, .. } => {
            (payload.kind().into(), text.clone(), Some(column.clone()))
        }
        ClaimPayload::ColumnRole { column, role, .. } => (
            payload.kind().into(),
            role.as_str().into(),
            Some(column.clone()),
        ),
        ClaimPayload::DefaultTimeColumn { column, .. } => {
            (payload.kind().into(), column.clone(), Some(column.clone()))
        }
        // `ClaimPayload` is `#[non_exhaustive]`; `Relationship` is not exposed on
        // the CLI in this slice and any future variant is handled here too. Both
        // render a stable kind with no value, leaking neither the target object
        // nor any payload field.
        _ => (payload.kind().into(), String::new(), None),
    }
}

fn conflict_view(conflict: &ContractConflict) -> ContractConflictView {
    ContractConflictView {
        kind: conflict.kind.to_string(),
        claim_ids: conflict
            .claim_ids
            .iter()
            .map(|id| id.as_str().to_string())
            .collect(),
    }
}

/// Maps one queued candidate to its render DTO. `profile_name` is the name the
/// adapter resolved — never the opaque identity, which has no field on the DTO.
/// The full claim id is carried unabbreviated because the reviewer's next action
/// (`contracts review <id>`) keys on it. A candidate always has a payload; the
/// `None` arm is a defensive fallback, not a path that should render.
pub(crate) fn queue_item_view(
    candidate: &QueuedCandidate,
    profile_name: &str,
) -> ContractQueueItemView {
    let (kind, value, column) = candidate
        .claim
        .payload
        .as_ref()
        .map(render_payload)
        .unwrap_or_else(|| (String::new(), String::new(), None));
    ContractQueueItemView {
        profile: profile_name.to_string(),
        claim_id: candidate.claim.id.as_str().to_string(),
        kind,
        value,
        column,
        object: candidate.claim.object.qualified_name(),
        schema_state: schema_state_str(candidate.schema_state),
        evidence_count: candidate.evidence_count,
    }
}
