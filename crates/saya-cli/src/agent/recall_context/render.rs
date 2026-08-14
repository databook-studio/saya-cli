//! Rendering recalled contracts into the block body — slice 2b-3b §3.4.
//!
//! Presentation-only and model-facing: no colour, no box drawing, stable
//! ordering, one line per claim. The body is untrusted; this layer passes it
//! through verbatim and must not pre-escape it — escaping is `history_context`'s
//! job (Phase 2a) and doing it twice would double-escape.
//!
//! A `stale` contract is **included but plainly labelled** (SPEC REVIEW):
//! excluding would discard the query-shaping signal this slice exists to
//! deliver. The marker is in-band on the header line so the model never reads
//! a stale claim as current.

use crate::connection::ConnectionRegistry;
use crate::contracts::{ContractSchemaState, RetrievedContract};
use saya_store::StoredClaim;
use saya_types::ClaimPayload;
use std::collections::HashMap;
use std::fmt::Write;

/// `identity.as_str() → profile name` for the connections that resolved. The
/// body carries the name, never the opaque identity (§3.6, non-negotiable).
pub(super) fn name_by_identity(registry: &ConnectionRegistry) -> HashMap<String, String> {
    registry
        .entries()
        .iter()
        .filter_map(|(name, entry)| {
            entry
                .profile_id
                .as_deref()
                .map(|id| (id.to_string(), name.to_string()))
        })
        .collect()
}

/// Renders the selected contracts as one compact, stable body for a model.
pub(super) fn render_body(
    contracts: &[RetrievedContract],
    name_of: &HashMap<String, String>,
) -> String {
    let mut out = String::new();
    for contract in contracts {
        let profile_name = name_of
            .get(contract.object.profile().as_str())
            .map(String::as_str)
            .unwrap_or("");
        let state = schema_state_token(contract.schema_state);
        // Header: qualified object, schema state, then the profile name.
        let _ = writeln!(
            out,
            "{obj}  [{state}]  (profile: {profile}){stale}",
            obj = contract.object.qualified_name(),
            state = state,
            profile = profile_name,
            stale = stale_note(contract.schema_state),
        );
        for claim in &contract.claims {
            let line = claim_line(claim);
            if !line.is_empty() {
                let _ = writeln!(out, "  {line}");
            }
        }
    }
    out
}

/// One claim rendered as `kind  value` (or `kind  column: value` when the
/// claim is column-scoped). No claim id or origin — the model needs the fact,
/// not the bookkeeping. **Status is the exception** (spec 4b §1): a `Candidate`
/// claim is prefixed with a fixed, in-band `[candidate — unconfirmed]` marker
/// so the model cannot read an inferred claim as an established fact (ADR 0002
/// §4 — inference is not confirmation). Confirmed claims render with no marker,
/// byte-identical to before this slice.
fn claim_line(claim: &StoredClaim) -> String {
    let Some(payload) = claim.payload.as_ref() else {
        return String::new();
    };
    let marker = candidate_marker(claim.status);
    match payload {
        ClaimPayload::TableDescription { text, .. } => {
            format!("{marker}{}  {text}", payload.kind())
        }
        ClaimPayload::TableAlias { alias, .. } => format!("{marker}{}  {alias}", payload.kind()),
        ClaimPayload::TableGrain { description, .. } => {
            format!("{marker}{}  {description}", payload.kind())
        }
        ClaimPayload::ColumnDescription { column, text, .. } => {
            format!("{marker}{}  {column}: {text}", payload.kind())
        }
        ClaimPayload::ColumnRole { column, role, .. } => {
            format!("{marker}{}  {column}: {}", payload.kind(), role.as_str())
        }
        ClaimPayload::DefaultTimeColumn { column, .. } => {
            format!("{marker}{}  {column}", payload.kind())
        }
        // `Relationship` is not exposed on the CLI in this slice; a future
        // variant is handled here too. No value leaks for an unknown shape.
        _ => format!("{marker}{}", payload.kind()),
    }
}

/// The in-band prefix that marks a claim as an unconfirmed candidate. Empty for
/// every confirmed claim (so the body is byte-identical to before this slice);
/// `[candidate — unconfirmed] ` for a `Candidate` claim. The marker is fixed and
/// in-band on the claim line, so a model cannot read an inferred claim as an
/// established fact (spec 4b §1, ADR 0002 §4). Every other status is excluded
/// from recall before it reaches the renderer, so it never produces a marker.
fn candidate_marker(status: saya_types::ClaimStatus) -> &'static str {
    match status {
        saya_types::ClaimStatus::Candidate => "[candidate — unconfirmed] ",
        _ => "",
    }
}

/// The stable, machine-ish token for a contract's aggregated schema state.
fn schema_state_token(state: ContractSchemaState) -> &'static str {
    match state {
        ContractSchemaState::Current => "current",
        ContractSchemaState::NeedsReview => "needs_review",
        ContractSchemaState::Stale => "stale",
        ContractSchemaState::LiveSchemaUnavailable => "live_schema_unavailable",
    }
}

/// A plain, in-band marker that a contract is possibly out of date. Appended to
/// the header line so a `stale` claim never reads as current. Empty for every
/// healthy state.
fn stale_note(state: ContractSchemaState) -> &'static str {
    match state {
        ContractSchemaState::Stale => "  — possibly out of date: a column it depends on is gone",
        _ => "",
    }
}
