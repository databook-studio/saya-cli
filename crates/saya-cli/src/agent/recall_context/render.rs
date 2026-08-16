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
//!
//! A **confirmed** claim binds (spec P2a): one stanza-level directive per
//! contract ([`CONFIRMED_DIRECTIVE`]) states the authority, and a `[confirmed] `
//! marker on each undisputed confirmed line ([`claim_line`]) makes the asymmetry
//! with `[candidate — unconfirmed] ` legible at the point of use. A disputed
//! confirmed claim shows `[disputed] ` and not `[confirmed] ` — a disagreement
//! never reads as a settled instruction.

use crate::connection::ConnectionRegistry;
use crate::contracts::{ContractSchemaState, RetrievedContract};
use saya_store::StoredClaim;
use saya_types::ClaimPayload;
use std::collections::{HashMap, HashSet};
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
        // P2a: one stanza-level directive per contract, before its claims, so
        // the model reads the authority policy then the facts it governs. The
        // directive is literally an instruction (it does not guarantee the model
        // obeys — structural detection is P2b), and it names "Confirmed" so the
        // `[confirmed]` marker and the `[candidate — unconfirmed]` marker on the
        // claim lines below point back at it. It is not worded as a per-claim
        // decoration: one line per stanza, bounded by `max_objects`, so it costs
        // the byte budget once per contract, not once per claim (spec P2a §3.5).
        let _ = writeln!(out, "  {CONFIRMED_DIRECTIVE}");
        // 5e: ids of the claims this contract's conflicts name, so each disputed
        // claim is marked in-band. Computed once per contract; empty (and thus a
        // no-op) when there is no conflict.
        let disputed: HashSet<String> = super::dispute::disputed_ids(&contract.conflicts);
        for claim in &contract.claims {
            let is_disputed = disputed.contains(claim.id.as_str());
            let line = claim_line(claim, is_disputed);
            if !line.is_empty() {
                let _ = writeln!(out, "  {line}");
            }
        }
        // 5e: one summary line per conflict plus the do-not-choose instruction.
        // Appended after the claim lines so a reader scanning the stanza sees every
        // disputed claim before the disagreement is named.
        out.push_str(&super::dispute::conflict_lines(contract));
    }
    out
}

/// The stanza-level directive that gives a confirmed claim its authority. One
/// line per contract, before the claims, so the `[confirmed]` / `[candidate —
/// unconfirmed]` markers below point back at it. Wording is an instruction, not
/// a guarantee: it does not say SAYA *will* use the facts (this slice cannot
/// enforce that — structural detection is P2b), only that a confirmed claim
/// binds and a departure must be named in the answer. "Confirmed" (not "your
/// facts") keeps it literally true for a `TeamFile`-imported confirmed claim a
/// teammate established, which "established by you" would not (spec P2a §3.3).
pub(super) const CONFIRMED_DIRECTIVE: &str = "Confirmed claims below bind: use them as given, and say in the answer when you depart from one.";

/// One claim rendered as `kind  value` (or `kind  column: value` when the
/// claim is column-scoped). No claim id or origin — the model needs the fact,
/// not the bookkeeping. **Status is the exception**: exactly one in-band marker
/// prefixes the line, chosen by [`authority_marker`] with precedence
/// `disputed` beats `confirmed` beats `candidate`. A `Candidate` carries
/// `[candidate — unconfirmed] ` so an inferred claim never reads as an
/// established fact (spec 4b §1, ADR 0002 §4). A `Confirmed`, undisputed claim
/// carries `[confirmed] ` — a binding fact, not advisory context (spec P2a §2).
/// A disputed claim carries `[disputed] ` and *not* `[confirmed] `, so two
/// contradictory confirmed claims never both read as one settled instruction
/// (spec 5e §1, P2a §3, Deliverable 2). Only `Confirmed` claims can be disputed
/// (`is_recallable` is `Confirmed`-only), so the precedence suppresses the
/// confirmed marker exactly where it must.
fn claim_line(claim: &StoredClaim, is_disputed: bool) -> String {
    let Some(payload) = claim.payload.as_ref() else {
        return String::new();
    };
    let marker = authority_marker(claim.status, is_disputed);
    // `claim_value` is the single source of the value/column a claim shows; the
    // prompt body and the P1a receipt both read it. `authority_marker` already
    // picks one in-band prefix (empty for an unknown status this slice never
    // admits), so no trailing space is added for a payload shape that renders
    // no value.
    let (column, value) = claim_value(payload);
    match (column.as_deref(), value.is_empty()) {
        (Some(col), false) => format!("{marker}{}  {col}: {value}", payload.kind()),
        (Some(_), true) => format!("{marker}{}", payload.kind()),
        (None, false) => format!("{marker}{}  {value}", payload.kind()),
        (None, true) => format!("{marker}{}", payload.kind()),
    }
}

/// The short rendered value for a claim, plus the column name when the claim is
/// column-scoped. This is the single source of truth for what a claim "shows"
/// as a value: the prompt body ([`claim_line`]) and the P1a recall receipt both
/// read it here, so the receipt can never name a value the prompt did not; the
/// propose tool reuses it so a `KnowledgeProposed` event names the same value.
/// `value` is `""` only for a payload shape this slice does not render (a
/// future variant); the `kind` still identifies it.
pub(crate) fn claim_value(payload: &ClaimPayload) -> (Option<String>, String) {
    match payload {
        ClaimPayload::TableDescription { text, .. } => (None, text.clone()),
        ClaimPayload::TableAlias { alias, .. } => (None, alias.clone()),
        ClaimPayload::TableGrain { description, .. } => (None, description.clone()),
        ClaimPayload::ColumnDescription { column, text, .. } => {
            (Some(column.clone()), text.clone())
        }
        ClaimPayload::ColumnRole { column, role, .. } => {
            (Some(column.clone()), role.as_str().to_string())
        }
        ClaimPayload::DefaultTimeColumn { column, .. } => (None, column.clone()),
        // `Relationship` is not exposed on the CLI in this slice; a future
        // variant is handled here too. No value leaks for an unknown shape.
        _ => (None, String::new()),
    }
}

/// The one in-band prefix a claim line carries, encoding the authority a claim
/// has at the point of use. Precedence is **disputed** beats **confirmed** beats
/// **candidate** (spec P2a §2/§3, 5e §1, 4b §1):
///
/// - A disputed claim (`is_disputed`) carries `[disputed] ` and nothing else,
///   so two contradictory confirmed claims never both read as binding. Only
///   `Confirmed` claims can be disputed (`is_recallable` is `Confirmed`-only),
///   so this arm suppresses the `[confirmed] ` marker a disputed confirmed
///   claim would otherwise carry — the dispute must win (Deliverable 2).
/// - A `Candidate` claim carries `[candidate — unconfirmed] `, so an inferred
///   claim never reads as an established fact (ADR 0002 §4).
/// - A `Confirmed`, undisputed claim carries `[confirmed] `, the marker the
///   stanza directive (see [`render_body`]) refers to: a binding fact, not
///   advisory context (spec P2a §2).
///
/// Empty for any status recall excludes before it reaches the renderer
/// (`Rejected`/`Stale`/`Contradicted`/`Forgotten`, and any future variant —
/// `ClaimStatus` is `#[non_exhaustive]`), so a non-recallable status produces
/// no marker and no trailing space.
fn authority_marker(status: saya_types::ClaimStatus, is_disputed: bool) -> &'static str {
    if is_disputed {
        return super::dispute::DISPUTE_MARKER;
    }
    match status {
        saya_types::ClaimStatus::Candidate => "[candidate — unconfirmed] ",
        saya_types::ClaimStatus::Confirmed => "[confirmed] ",
        // These never reach the renderer: recall admits `Confirmed`, and
        // `IncludeCandidates` admits `Candidate` too. Every other status is
        // excluded upstream, so an empty marker is the only honest rendering.
        saya_types::ClaimStatus::Rejected
        | saya_types::ClaimStatus::Stale
        | saya_types::ClaimStatus::Contradicted
        | saya_types::ClaimStatus::Forgotten => "",
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
