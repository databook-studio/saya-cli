//! Text shaping for [`AgentEvent::KnowledgeSupplied`] — spec P1c.
//!
//! The wording here is a correctness constraint, not style: the line names
//! **claims** (never *facts*) and says **supplied** (never *applied* or *used*).
//! Recall injects context; the model may ignore it — a confirmed claim being
//! supplied does not mean the generated SQL honoured it (we have measured that
//! it frequently does not). A causal verb would overstate what we know, in the
//! one feature whose pitch is that it does not overstate.
//!
//! This shaper is shared by the text/CLI path ([`super::TerminalEvent`] →
//! [`super::render_event`]) and the TUI path ([`crate::interactive::tui`]
//! `apply_event`) so the wording lives in one place. Both adapters lead — the
//! line renders when the event arrives, before the answer streams (spec §6).

use saya_agent::{KnowledgeOutcome, SuppliedClaimDto, SuppliedContractDto};
use saya_types::ClaimStatus;

/// Shapes the full text block for one `KnowledgeSupplied` event, for any
/// adapter that prints it. Returns an empty string when the event should be
/// silent (see `Ran`-and-found-nothing below); callers may treat empty as
/// "render nothing."
///
/// The three outcomes read differently (spec §4):
/// - [`KnowledgeOutcome::Off`] — recall is disabled by config; one plain line.
/// - [`KnowledgeOutcome::Skipped`] — the privacy gate closed; one plain line,
///   distinct from `Off`.
/// - [`KnowledgeOutcome::Ran`] — recall ran. With claims, the compact header
///   plus one line per claim (the "inspectable" form, spec §3). With nothing
///   found and nothing dropped, **silent** — a defensible decision (spec §4):
///   "nothing was supplied" is the default expectation, and silence keeps the
///   rare informative lines salient. `store_unavailable` is *not* silence: a
///   store failure degraded recall, and hiding it would collapse "couldn't read
///   your memory" into "your memory is empty". `dropped_by_bounds > 0` is never
///   silent either — a non-zero dropped count is the event saying "the list is
///   a subset, not the whole" (spec §4), and it overrides the silence decision.
pub(crate) fn knowledge_supplied_text(
    outcome: KnowledgeOutcome,
    contracts: &[SuppliedContractDto],
    dropped_by_bounds: usize,
) -> String {
    match outcome {
        KnowledgeOutcome::Off => "memory off · recall disabled\n".into(),
        KnowledgeOutcome::Skipped => "memory skipped · not permitted to read saved claims\n".into(),
        KnowledgeOutcome::Ran { store_unavailable } => {
            if store_unavailable {
                // Distinct from Ran-nothing (silent) and from Skipped/Off: recall
                // ran, but the store could not be read. Quiet — not an error.
                return "memory supplied · store unavailable — recall could not read saved claims\n".into();
            }
            let total: usize = contracts.iter().map(|c| c.claims.len()).sum();
            if total == 0 && dropped_by_bounds == 0 {
                // Ran and matched nothing, nothing dropped: silent (spec §4).
                return String::new();
            }
            let unconfirmed = count_unconfirmed(contracts);
            let mut out = String::from("memory supplied · ");
            // "1 claims" in the one line that tells a user what SAYA assumed reads
            // as carelessness, in the feature whose whole pitch is careful wording.
            out.push_str(&format!(
                "{total} claim{}",
                if total == 1 { "" } else { "s" }
            ));
            if unconfirmed > 0 {
                out.push_str(&format!(" ({unconfirmed} unconfirmed)"));
            }
            if dropped_by_bounds > 0 {
                out.push_str(&format!(" · {dropped_by_bounds} more dropped by bounds"));
            }
            out.push('\n');
            for contract in contracts {
                out.push_str(&contract_header(contract));
                for claim in &contract.claims {
                    out.push_str(&claim_line(claim));
                }
            }
            out
        }
        // `KnowledgeOutcome` is #[non_exhaustive]: a future variant this shaper
        // does not know about must not panic (spec §4). Render nothing rather
        // than guess — the turn still completes (rendering never fails it).
        _ => String::new(),
    }
}

/// Counts claims whose status is not `Confirmed`. On the supply path only
/// `Confirmed` and `Candidate` appear, but `ClaimStatus` is `#[non_exhaustive]`
/// — any future status reads as unconfirmed, which is the cautious mark.
fn count_unconfirmed(contracts: &[SuppliedContractDto]) -> usize {
    contracts
        .iter()
        .flat_map(|c| c.claims.iter())
        .filter(|claim| !is_confirmed(claim.status))
        .count()
}

fn is_confirmed(status: ClaimStatus) -> bool {
    matches!(status, ClaimStatus::Confirmed)
}

/// One object's header line: the qualified name, the schema state token, and
/// the profile name (never the opaque identity — the DTO has no such field).
fn contract_header(contract: &SuppliedContractDto) -> String {
    format!(
        "  {}  [{}]  (profile: {})\n",
        contract.object, contract.schema_state, contract.profile
    )
}

/// One claim line: kind, value, an optional column, and the status word —
/// with an explicit `unconfirmed` mark when the status is not `Confirmed`
/// (spec §4: unconfirmed claims are marked wherever claims are shown).
fn claim_line(claim: &SuppliedClaimDto) -> String {
    let kind = claim.kind.as_str();
    let value = claim.value.as_str();
    let column = match &claim.column {
        Some(column) => format!("  col:{column}"),
        None => String::new(),
    };
    let status = claim.status.as_str();
    let mark = if is_confirmed(claim.status) {
        String::new()
    } else {
        "  (unconfirmed)".to_string()
    };
    format!("    {kind}  {value}{column}  {status}{mark}\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use saya_agent::{AgentEvent, KnowledgeOutcome, SuppliedClaimDto, SuppliedContractDto};
    use saya_types::{ClaimId, ClaimStatus};

    fn claim(
        id: &str,
        kind: &str,
        value: &str,
        column: Option<&str>,
        status: ClaimStatus,
    ) -> SuppliedClaimDto {
        SuppliedClaimDto {
            claim_id: ClaimId::parse(id).unwrap(),
            kind: kind.into(),
            value: value.into(),
            column: column.map(str::to_string),
            status,
        }
    }

    fn contract(
        profile: &str,
        object: &str,
        state: &str,
        claims: Vec<SuppliedClaimDto>,
    ) -> SuppliedContractDto {
        SuppliedContractDto {
            profile: profile.into(),
            object: object.into(),
            schema_state: state.into(),
            claims,
        }
    }

    /// `KnowledgeOutcome::Ran { store_unavailable }` is a struct variant;
    /// a local constructor reads more clearly at the call sites than the brace
    /// form repeated in every fixture.
    fn ran(store_unavailable: bool) -> KnowledgeOutcome {
        KnowledgeOutcome::Ran { store_unavailable }
    }

    /// The three outcomes render distinguishably (spec §5).
    #[test]
    fn the_three_outcomes_render_distinguishably() {
        let off = knowledge_supplied_text(KnowledgeOutcome::Off, &[], 0);
        let skipped = knowledge_supplied_text(KnowledgeOutcome::Skipped, &[], 0);
        let ran_empty = knowledge_supplied_text(ran(false), &[], 0);
        let ran_found = knowledge_supplied_text(
            ran(false),
            &[contract(
                "analytics",
                "catalog.public.orders",
                "current",
                vec![claim(
                    "c-1",
                    "default_time_column",
                    "created_at",
                    Some("created_at"),
                    ClaimStatus::Confirmed,
                )],
            )],
            0,
        );
        assert_eq!(off, "memory off · recall disabled\n");
        assert_eq!(
            skipped,
            "memory skipped · not permitted to read saved claims\n"
        );
        // Ran-and-found-nothing is silent (the decision), distinct from Off and
        // Skipped which each print a line — so the three states read differently.
        assert_eq!(ran_empty, "");
        assert!(ran_found.starts_with("memory supplied · 1 claim"));
        assert!(
            !ran_found.contains("1 claims"),
            "a single claim must not read as a plural: {ran_found}"
        );
        // All three states are mutually distinguishable on the rendered output.
        assert_ne!(off, skipped);
        assert_ne!(off, ran_empty);
        assert_ne!(skipped, ran_empty);
        assert_ne!(ran_empty, ran_found);
    }

    /// A candidate claim is marked, both in the count and on its line (spec §5).
    #[test]
    fn a_candidate_claim_is_marked() {
        let text = knowledge_supplied_text(
            ran(false),
            &[contract(
                "analytics",
                "catalog.public.orders",
                "current",
                vec![
                    claim(
                        "c-1",
                        "default_time_column",
                        "created_at",
                        Some("created_at"),
                        ClaimStatus::Confirmed,
                    ),
                    claim("c-2", "table_alias", "orders", None, ClaimStatus::Candidate),
                ],
            )],
            0,
        );
        // The compact header counts the unconfirmed claim.
        assert!(
            text.contains("memory supplied · 2 claims (1 unconfirmed)"),
            "{text}"
        );
        // The candidate's line carries the explicit unconfirmed mark; the
        // confirmed claim's line does not.
        assert!(
            text.contains("table_alias  orders  candidate  (unconfirmed)"),
            "{text}"
        );
        assert!(
            text.contains("default_time_column  created_at  col:created_at  confirmed\n"),
            "{text}"
        );
    }

    /// A non-zero dropped count is shown on the header (spec §5 / §4).
    #[test]
    fn a_nonzero_dropped_count_is_shown() {
        // With claims kept and some dropped.
        let text = knowledge_supplied_text(
            ran(false),
            &[contract(
                "analytics",
                "catalog.public.orders",
                "current",
                vec![claim(
                    "c-1",
                    "table_alias",
                    "orders",
                    None,
                    ClaimStatus::Confirmed,
                )],
            )],
            30,
        );
        assert!(text.contains("· 30 more dropped by bounds"), "{text}");

        // All claims dropped by bounds: supplied is empty but dropped > 0 — this
        // is NOT silence; the dropped count is the signal (spec §4).
        let all_dropped = knowledge_supplied_text(ran(false), &[], 30);
        assert!(
            all_dropped.contains("memory supplied · 0 claims"),
            "{all_dropped}"
        );
        assert!(
            all_dropped.contains("· 30 more dropped by bounds"),
            "{all_dropped}"
        );
    }

    /// `store_unavailable` renders distinctly and quietly — not silence, not an
    /// error — and stays distinguishable from Ran-nothing and Skipped (Gap).
    #[test]
    fn store_unavailable_renders_distinctly() {
        let store_down = knowledge_supplied_text(ran(true), &[], 0);
        let ran_empty = knowledge_supplied_text(ran(false), &[], 0);
        let skipped = knowledge_supplied_text(KnowledgeOutcome::Skipped, &[], 0);
        assert!(
            store_down.contains("memory supplied · store unavailable"),
            "{store_down}"
        );
        assert_ne!(store_down, ran_empty);
        assert_ne!(store_down, skipped);
    }

    /// No opaque profile identity reaches output: the human-facing name appears,
    /// a fabricated identity string does not (spec §5 / §4). The DTO has no
    /// identity field by construction; this asserts the rendered text inherits
    /// that guarantee.
    #[test]
    fn no_opaque_identity_reaches_output() {
        let fake_identity =
            "sha256:9f2a8c7b1e4d0a6f3c5b8e2d7a9f1c4b6e8a0d2f4c6b8e0a2d4f6c8b0e2d4f6";
        let text = knowledge_supplied_text(
            ran(false),
            &[contract(
                "analytics",
                "catalog.public.orders",
                "current",
                vec![claim(
                    "c-1",
                    "table_alias",
                    "orders",
                    None,
                    ClaimStatus::Confirmed,
                )],
            )],
            0,
        );
        assert!(text.contains("analytics"), "profile name appears: {text}");
        assert!(
            !text.contains(fake_identity),
            "opaque identity leaked: {text}"
        );
    }

    /// The event serializes under its `knowledge_supplied` type tag (JSON/NDJSON
    /// adapters fall out of the serde derive on `TerminalEvent`).
    #[test]
    fn event_serializes_with_type_tag() {
        let event = AgentEvent::knowledge_supplied(
            ran(false),
            vec![contract(
                "analytics",
                "catalog.public.orders",
                "current",
                vec![claim(
                    "c-1",
                    "table_alias",
                    "orders",
                    None,
                    ClaimStatus::Candidate,
                )],
            )],
            0,
        );
        let json = serde_json::to_string(&event).expect("serializes");
        assert!(json.contains(r#""type":"knowledge_supplied""#), "{json}");
        assert!(json.contains("supplied"), "{json}");
    }
}
