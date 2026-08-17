//! Assembling selected candidates into bounded `RetrievedContract`s.
//!
//! Split from `recall.rs` by concern: `recall` decides what to ask for and how
//! to fail; this module applies the per-object and per-item **count** bounds
//! and builds the typed contracts. The byte bound is *not* applied here: it
//! belongs to the rendered block the caller sends, and what reaches the request
//! is the rendered body (headers, markers, conflict lines, the wrapper), not
//! serialized payloads. Measuring payloads here would bound the wrong unit, and
//! the count-bounded contracts this returns are trimmed against the rendered
//! body — and the agent message budget — by the prompt-recall caller. See
//! `crate::agent::recall_context`.
//!
//! This module computes each contract's schema state (including `Stale`, the
//! `Invalid` verdict under D-4's name) and returns the contract regardless of
//! state — it does **not** decide who sees it. That decision is
//! [`super::retrieval`]'s: a `Stale` contract is dropped for the model and kept
//! for human review, in one place rather than per adapter.
//!
//! Validity is [`super::knowledge_validity::item_validity_for`] — the D-4
//! binding model. A fact depends only on what its `SchemaBinding` names, so an
//! unrelated column changing never invalidates; a gone or retyped bound column
//! reads `Invalid` (→ `Stale`), and an absent/unreadable schema reads
//! `SchemaUnavailable`, never `Invalid`.

use crate::contracts::availability::{SchemaAvailability, SchemaFreshness};
use crate::contracts::conflict::conflicts_for;
use crate::contracts::knowledge_validity::item_validity_for;
use crate::contracts::selection::Candidate;
use crate::contracts::view::{
    ContractClaim, ContractSchemaState, RecallDiagnostics, RetrievedContract,
};
use saya_types::ProfileIdentity;

/// Turns the ranked candidates into bounded contracts, recording diagnostics.
///
/// Only the count bounds (`max_objects`, `max_claims_per_object`) are applied
/// here. `max_bytes` is the caller's concern — it bounds the rendered block, not
/// the serialized payloads this layer sees, so it is enforced where the body is
/// rendered (the prompt-recall path).
pub(crate) fn assemble(
    candidates: &[Candidate],
    schemas: &[(ProfileIdentity, SchemaAvailability)],
    bounds: super::RecallBounds,
    freshness: SchemaFreshness,
    diag: &mut RecallDiagnostics,
) -> Vec<RetrievedContract> {
    let mut out: Vec<RetrievedContract> = Vec::new();
    for candidate in candidates {
        if out.len() >= bounds.max_objects {
            // Object-bound truncation: further matches were dropped. Flag the
            // boundary contract so the caller never mistakes a partial result
            // for a complete one, and count the dropped objects' items as a
            // *bound* drop (not a schema-policy drop — those are unrelated and
            // counted in `excluded_by_schema`). Each dropped object's item
            // count is its actual selection count, not the per-object cap.
            if let Some(last) = out.last_mut() {
                last.truncated = true;
            }
            for dropped in &candidates[out.len()..] {
                diag.excluded_by_count_bounds += dropped.items.len();
            }
            break;
        }
        out.push(build_contract(candidate, schemas, bounds, freshness, diag));
    }
    out
}

fn build_contract(
    candidate: &Candidate,
    schemas: &[(ProfileIdentity, SchemaAvailability)],
    bounds: super::RecallBounds,
    freshness: SchemaFreshness,
    diag: &mut RecallDiagnostics,
) -> RetrievedContract {
    let mut items: Vec<_> = candidate.items.clone();
    let mut truncated = false;
    if items.len() > bounds.max_claims_per_object {
        // Per-object item bound: the tail is dropped, not gone silently. Count
        // the dropped items as a bound drop so the receipt's `dropped_by_bounds`
        // can say the contract is a subset.
        diag.excluded_by_count_bounds += items.len() - bounds.max_claims_per_object;
        items.truncate(bounds.max_claims_per_object);
        truncated = true;
    }
    let live = schemas
        .iter()
        .find(|(p, _)| p == candidate.object.profile())
        .map(|(_, avail)| avail);
    let availability = live.unwrap_or(&SchemaAvailability::Missing);
    // Aggregate the per-item D-4 verdict into the contract's schema state. The
    // `ContractClaim` projection (for render/receipt/conflict) is built beside
    // the verdict so a row that fails to parse its id is dropped here rather
    // than surfacing a half-built carrier downstream.
    let mut claims: Vec<ContractClaim> = Vec::with_capacity(items.len());
    let mut state = ContractSchemaState::Current;
    for item in &items {
        let validity = item_validity_for(item, availability, freshness);
        state = state.aggregate(validity.into());
        if let Some(carrier) = ContractClaim::from_knowledge_item(item) {
            claims.push(carrier);
        }
    }
    let conflicts = conflicts_for(&claims);
    RetrievedContract {
        object: candidate.object.clone(),
        schema_state: state,
        claims,
        conflicts,
        truncated,
    }
}
