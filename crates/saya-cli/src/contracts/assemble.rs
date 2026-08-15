//! Assembling selected candidates into bounded `RetrievedContract`s.
//!
//! Split from `recall.rs` by concern: `recall` decides what to ask for and how
//! to fail; this module applies the per-object and per-byte bounds and builds
//! the typed contracts. `max_bytes` is enforced over the serialized claim
//! payloads actually selected, before returning, so a caller never receives
//! more than it asked for.
//!
//! This module computes each contract's schema state (including `Stale`) and
//! returns the contract regardless of state — it does **not** decide who sees
//! it. That decision is [`super::retrieval`]'s: a `Stale` contract is dropped
//! for the model and kept for human review, in one place rather than per
//! adapter.

use crate::contracts::conflict::conflicts_for;
use crate::contracts::selection::Candidate;
use crate::contracts::validity::schema_state_for;
use crate::contracts::view::{ContractSchemaState, RecallDiagnostics, RetrievedContract};
use saya_store::StoredClaim;
use saya_types::{ProfileIdentity, SchemaTree};

/// Turns the ranked candidates into bounded contracts, recording diagnostics.
pub(crate) fn assemble(
    candidates: &[Candidate],
    schemas: &[(ProfileIdentity, SchemaTree)],
    bounds: super::RecallBounds,
    diag: &mut RecallDiagnostics,
) -> Vec<RetrievedContract> {
    let mut out: Vec<RetrievedContract> = Vec::new();
    let mut bytes = 0usize;
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
        let Some(contract) = build_contract(candidate, schemas, bounds, &mut bytes, diag) else {
            continue;
        };
        out.push(contract);
    }
    out
}

fn build_contract(
    candidate: &Candidate,
    schemas: &[(ProfileIdentity, SchemaTree)],
    bounds: super::RecallBounds,
    bytes: &mut usize,
    diag: &mut RecallDiagnostics,
) -> Option<RetrievedContract> {
    let mut claims: Vec<StoredClaim> = candidate.claims.clone();
    let mut truncated = false;
    if claims.len() > bounds.max_claims_per_object {
        claims.truncate(bounds.max_claims_per_object);
        truncated = true;
    }
    let mut kept = Vec::new();
    for claim in claims {
        let size = serde_json::to_string(&claim.payload)
            .map(|s| s.len())
            .unwrap_or(0);
        if *bytes + size > bounds.max_bytes && !kept.is_empty() {
            truncated = true;
            break;
        }
        *bytes += size;
        kept.push(claim);
    }
    if kept.is_empty() {
        diag.excluded_by_schema += 1;
        return None;
    }
    let live = schemas
        .iter()
        .find(|(p, _)| p == candidate.object.profile())
        .map(|(_, tree)| tree);
    let state = kept
        .iter()
        .map(|c| schema_state_for(c, live))
        .fold(ContractSchemaState::Current, |acc, s| acc.aggregate(s));
    let conflicts = conflicts_for(&kept);
    Some(RetrievedContract {
        object: candidate.object.clone(),
        schema_state: state,
        claims: kept,
        conflicts,
        truncated,
    })
}
