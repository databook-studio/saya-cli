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
/// claim is column-scoped). No claim id, origin, or status — the model needs
/// the fact, not the bookkeeping.
fn claim_line(claim: &StoredClaim) -> String {
    let Some(payload) = claim.payload.as_ref() else {
        return String::new();
    };
    match payload {
        ClaimPayload::TableDescription { text, .. } => format!("{}  {text}", payload.kind()),
        ClaimPayload::TableAlias { alias, .. } => format!("{}  {alias}", payload.kind()),
        ClaimPayload::TableGrain { description, .. } => {
            format!("{}  {description}", payload.kind())
        }
        ClaimPayload::ColumnDescription { column, text, .. } => {
            format!("{}  {column}: {text}", payload.kind())
        }
        ClaimPayload::ColumnRole { column, role, .. } => {
            format!("{}  {column}: {}", payload.kind(), role.as_str())
        }
        ClaimPayload::DefaultTimeColumn { column, .. } => {
            format!("{}  {column}", payload.kind())
        }
        // `Relationship` is not exposed on the CLI in this slice; a future
        // variant is handled here too. No value leaks for an unknown shape.
        _ => payload.kind().to_string(),
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
