//! Assembling selected candidates into bounded `RetrievedContract`s.
//!
//! Split from `recall.rs` by concern: `recall` decides what to ask for and how
//! to fail; this module applies the per-object and per-claim **count** bounds
//! and builds the typed contracts. The byte bound is *not* applied here: it
//! belongs to the rendered block the caller sends, and what reaches the request
//! is the rendered body (headers, markers, conflict lines, the wrapper), not
//! serialized payloads. Measuring payloads here would bound the wrong unit, and
//! the count-bounded contracts this returns are trimmed against the rendered
//! body — and the agent message budget — by the prompt-recall caller. See
//! `crate::agent::recall_context`.
//!
//! This module computes each contract's schema state (including `Stale`) and
//! returns the contract regardless of state — it does **not** decide who sees
//! it. That decision is [`super::retrieval`]'s: a `Stale` contract is dropped
//! for the model and kept for human review, in one place rather than per
//! adapter.

use crate::contracts::availability::{SchemaAvailability, SchemaFreshness};
use crate::contracts::conflict::conflicts_for;
use crate::contracts::selection::Candidate;
use crate::contracts::validity::schema_state_for;
use crate::contracts::view::{ContractSchemaState, RecallDiagnostics, RetrievedContract};
use saya_store::StoredClaim;
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
            // for a complete one.
            if let Some(last) = out.last_mut() {
                last.truncated = true;
            }
            diag.excluded_by_schema += candidates.len().saturating_sub(out.len());
            break;
        }
        out.push(build_contract(candidate, schemas, bounds, freshness));
    }
    out
}

fn build_contract(
    candidate: &Candidate,
    schemas: &[(ProfileIdentity, SchemaAvailability)],
    bounds: super::RecallBounds,
    freshness: SchemaFreshness,
) -> RetrievedContract {
    let mut claims: Vec<StoredClaim> = candidate.claims.clone();
    let mut truncated = false;
    if claims.len() > bounds.max_claims_per_object {
        claims.truncate(bounds.max_claims_per_object);
        truncated = true;
    }
    let live = schemas
        .iter()
        .find(|(p, _)| p == candidate.object.profile())
        .map(|(_, avail)| avail);
    let state = claims
        .iter()
        .map(|c| schema_state_for(c, live.unwrap_or(&SchemaAvailability::Missing), freshness))
        .fold(ContractSchemaState::Current, |acc, s| acc.aggregate(s));
    let conflicts = conflicts_for(&claims);
    RetrievedContract {
        object: candidate.object.clone(),
        schema_state: state,
        claims,
        conflicts,
        truncated,
    }
}
