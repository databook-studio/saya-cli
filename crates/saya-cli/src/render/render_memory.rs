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

use saya_agent::{
    KnowledgeOutcome, LearningSkipReason, OverrideFindingDto, SuppliedClaimDto, SuppliedContractDto,
};
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
                // S28: the recall path points at the same next step the learn
                // path does (`unconfirmed, review with /queue`). A measured
                // store held 26 unconfirmed claims the user was told about
                // without being told where to act on them; the pointer rides
                // the unconfirmed count so a recall that found nothing — or
                // found only confirmed claims — stays silent (spec §4).
                out.push_str(&format!(
                    " ({unconfirmed} unconfirmed — review with /queue)"
                ));
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

/// One claim line: the short claim-id prefix (the `ki-xxxx` the user types into
/// `/confirm`/`/reject`/`/use`), kind, value, an optional column, and the status
/// word — with an explicit `unconfirmed` mark when the status is not `Confirmed`
/// (spec §4: unconfirmed claims are marked wherever claims are shown). The id
/// prefix is shown here so a user can act on the claim from the turn that just
/// displayed it, without finding and copying a 64-character id (spec D).
fn claim_line(claim: &SuppliedClaimDto) -> String {
    let id = abbreviate_id(claim.claim_id.as_str());
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
    format!("    {id}  {kind}  {value}{column}  {status}{mark}\n")
}

/// Abbreviation for the on-screen claim-id reference: the first six chars + `…`
/// when the id is longer. Mirrors `render_contract::abbreviate_id`'s width so
/// the memory receipt and `contracts list` show the same short reference a
/// `/confirm` prefix can match. The full id is never needed here — the prefix
/// is the reference, and the resolve step matches by leading chars.
fn abbreviate_id(id: &str) -> String {
    const PREFIX: usize = 6;
    if id.len() > PREFIX + 1 {
        format!("{}…", &id[..PREFIX])
    } else {
        id.to_string()
    }
}

/// Shapes the text block for one [`AgentEvent::KnowledgeOverridden`] event (spec
/// A1), for any adapter that prints it. Returns an empty string when there are
/// no findings; callers may treat empty as "render nothing."
///
/// The wording is a correctness constraint, not style, and the one the spec
/// checks hardest: the line says the SQL **referenced** columns, never that it
/// **used** them as the time column. The extractor (`sql_references`) cannot
/// tell a predicate from a projection, so the finding asserts only that these
/// columns were referenced where the claim named a different one — "SAYA used
/// rental_date as the time column" would assert a role the names do not prove.
/// The line names what the claim specified (`where you specified Y`) and the
/// short claim-id prefix the user can act on, matching the supplied-claim line.
///
/// Like the supplied shaper, this is shared by the text/CLI path and the TUI
/// path so the wording lives in one place. It trails the answer (emitted after
/// the loop), where a "the SQL contradicted a confirmed claim" notice belongs.
pub(crate) fn knowledge_overridden_text(findings: &[OverrideFindingDto]) -> String {
    if findings.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "memory overridden · {n} finding{s}\n",
        n = findings.len(),
        s = if findings.len() == 1 { "" } else { "s" }
    );
    for finding in findings {
        out.push_str(&finding_line(finding));
    }
    out
}

/// One finding line: the short claim-id prefix, the columns the SQL
/// **referenced**, what the claim specified, and the kind — never an asserted
/// "used" column. The observed columns are joined with a comma; the detector
/// sorts and dedupes them, so the order is stable.
fn finding_line(finding: &OverrideFindingDto) -> String {
    let id = abbreviate_id(finding.claim_id.as_str());
    let observed = finding.observed_columns.join(", ");
    format!(
        "    {id}  referenced {observed}  where you specified {claimed}  ({kind})\n",
        claimed = finding.claimed_value,
        kind = finding.kind,
    )
}

/// Shapes the text line for one [`AgentEvent::KnowledgeLearningSkipped`]
/// event (spec packet-54), for any adapter that prints it. Trails the answer —
/// the runtime emits it after the loop — so it lands below the assistant text,
/// where "and I did not learn from this turn" belongs.
///
/// Wording mirrors the recall precedent (`memory off · recall disabled`,
/// `memory skipped · not permitted…`): the line says plainly that nothing was
/// recorded and why, and that *this turn* was not learned from — not that
/// memory is broken. A timeout and a failure read differently so a user (and a
/// review) can tell them apart without re-deriving the outcome. The two
/// strings are fixed by the spec; this shaper exists so the headless path and
/// the TUI cannot disagree, the same arrangement `knowledge_supplied_text`
/// uses. `LearningSkipReason` is `#[non_exhaustive]`: a future variant this
/// shaper does not know about renders a generic line rather than panic.
pub(crate) fn learning_skipped_text(reason: LearningSkipReason) -> String {
    match reason {
        LearningSkipReason::TimedOut => {
            "memory not recorded · extraction timed out; this turn was not learned from\n".into()
        }
        LearningSkipReason::Failed => {
            "memory not recorded · extraction failed; this turn was not learned from\n".into()
        }
        // A future skip reason this shaper does not yet know about: name it as a
        // skip without guessing the cause. The turn still completes (rendering
        // never fails it).
        _ => "memory not recorded · this turn was not learned from\n".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use saya_agent::{
        AgentEvent, KnowledgeOutcome, OverrideFindingDto, SuppliedClaimDto, SuppliedContractDto,
    };
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
        // The compact header counts the unconfirmed claim and points at the review
        // queue — the same next step the learn path names (S28 folded-in).
        assert!(
            text.contains("memory supplied · 2 claims (1 unconfirmed — review with /queue)"),
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

    /// S28 folded-in: the `/queue` pointer rides only the unconfirmed count. A
    /// recall that supplied only confirmed claims must not nag, and the pre-existing
    /// silence rule stands — a `Ran`-and-found-nothing recall renders nothing at
    /// all (spec §4), so there is no pointer when there is nothing to point at.
    #[test]
    fn the_queue_pointer_rides_only_the_unconfirmed_count() {
        let confirmed_only = knowledge_supplied_text(
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
        assert!(
            confirmed_only.contains("1 claim") && !confirmed_only.contains("/queue"),
            "no pointer when everything is confirmed: {confirmed_only}"
        );
        // The silence rule still holds: found nothing, dropped nothing → empty.
        assert_eq!(knowledge_supplied_text(ran(false), &[], 0), "");
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

    /// Each claim line shows the short claim-id prefix (spec D): it is the
    /// reference a user types into `/confirm`/`/reject`/`/use` to act on the
    /// claim from this turn. The full 64-char id never appears — the prefix is
    /// the reference, and `abbreviate_id` keeps it to the same width
    /// `contracts list` shows.
    #[test]
    fn claim_line_shows_the_short_id_prefix_not_the_full_id() {
        // A realistic 67-char id (`c-` + 64 hex). `abbreviate_id` keeps the
        // first 6 chars + `…`.
        let long_id = "c-a86a3f0e9d7c5b4a2f0e9d7c5b4a2f0e9d7c5b4a2f0e9d7c5b4a2f0e9d7c5b4a";
        let text = knowledge_supplied_text(
            ran(false),
            &[contract(
                "analytics",
                "catalog.public.orders",
                "current",
                vec![claim(
                    long_id,
                    "table_alias",
                    "orders",
                    None,
                    ClaimStatus::Candidate,
                )],
            )],
            0,
        );
        assert!(
            text.contains("c-a86a…"),
            "short prefix must appear so the user can type it: {text}"
        );
        assert!(
            !text.contains(&long_id[7..]),
            "the full id beyond the prefix must not appear: {text}"
        );
    }

    // --- Spec A1: the KnowledgeOverridden shaper. ---

    fn override_finding(id: &str, claimed: &str, observed: &[&str]) -> OverrideFindingDto {
        OverrideFindingDto {
            claim_id: ClaimId::parse(id).unwrap(),
            kind: "default_time_column".into(),
            claimed_value: claimed.into(),
            observed_columns: observed.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Empty findings render nothing (spec A1 §3: "if it returns nothing, say
    /// nothing").
    #[test]
    fn no_findings_render_nothing() {
        assert_eq!(knowledge_overridden_text(&[]), "");
    }

    /// One finding renders the header and one line that names the referenced
    /// column and what the claim specified (spec A1 §6).
    #[test]
    fn one_finding_names_the_referenced_column_and_the_specified_value() {
        let text = knowledge_overridden_text(&[override_finding(
            "c-rental-time",
            "return_date",
            &["rental_date"],
        )]);
        assert!(
            text.contains("memory overridden · 1 finding"),
            "header: {text}"
        );
        assert!(
            text.contains("referenced rental_date"),
            "names the column the SQL referenced: {text}"
        );
        assert!(
            text.contains("where you specified return_date"),
            "names what the claim specified: {text}"
        );
        assert!(
            text.contains("default_time_column"),
            "names the kind: {text}"
        );
    }

    /// Multiple findings pluralize the header and render one line each.
    #[test]
    fn multiple_findings_pluralize_the_header() {
        let text = knowledge_overridden_text(&[
            override_finding("c-a", "return_date", &["rental_date"]),
            override_finding("c-b", "created_at", &["updated_at"]),
        ]);
        assert!(
            text.contains("memory overridden · 2 findings"),
            "pluralized header: {text}"
        );
        assert!(text.contains("referenced rental_date"), "{text}");
        assert!(text.contains("referenced updated_at"), "{text}");
    }

    // Test 5: the rendered text does not contain "used" as a causal assertion
    // about the time column. The extractor cannot tell a predicate from a
    // projection, so the line says "referenced", never "used". This is the
    // wording constraint the spec checks hardest.
    #[test]
    fn the_rendered_text_does_not_assert_the_model_used_a_time_column() {
        let text = knowledge_overridden_text(&[override_finding(
            "c-rental-time",
            "return_date",
            &["rental_date"],
        )]);
        // The line says the SQL *referenced* a column, never that SAYA *used*
        // one as the time column — the role is unknowable from names.
        assert!(
            !text.contains("used"),
            "the shaper must not assert a causal 'used' about the time column: {text}"
        );
        assert!(
            text.contains("referenced"),
            "the shaper says 'referenced': {text}"
        );
    }

    /// The event serializes under its `knowledge_overridden` type tag and never
    /// carries an opaque identity (the DTO has no such field, by construction).
    #[test]
    fn overridden_event_serializes_with_type_tag_and_no_identity() {
        let event = AgentEvent::knowledge_overridden(vec![override_finding(
            "c-rental-time",
            "return_date",
            &["rental_date"],
        )]);
        let json = serde_json::to_string(&event).expect("serializes");
        assert!(
            json.contains(r#""type":"knowledge_overridden""#),
            "type tag: {json}"
        );
        let fake_identity =
            "sha256:9f2a8c7b1e4d0a6f3c5b8e2d7a9f1c4b6e8a0d2f4c6b8e0a2d4f6c8b0e2d4f6";
        assert!(
            !json.contains(fake_identity),
            "opaque identity leaked into the event: {json}"
        );
    }

    // --- Spec packet-54: the KnowledgeLearningSkipped shaper. ---

    /// The two reasons render the exact spec strings and read differently — a
    /// timeout and a failure must not collapse (packet-54 decision 3).
    #[test]
    fn the_two_skip_reasons_render_the_spec_strings_and_differ() {
        use saya_agent::LearningSkipReason;
        let timed_out = learning_skipped_text(LearningSkipReason::TimedOut);
        let failed = learning_skipped_text(LearningSkipReason::Failed);
        assert_eq!(
            timed_out,
            "memory not recorded · extraction timed out; this turn was not learned from\n",
        );
        assert_eq!(
            failed,
            "memory not recorded · extraction failed; this turn was not learned from\n",
        );
        assert_ne!(timed_out, failed);
        // Both name what happened plainly, without claiming memory is broken.
        assert!(timed_out.contains("memory not recorded"));
        assert!(failed.contains("memory not recorded"));
        assert!(timed_out.contains("not learned from"));
        assert!(failed.contains("not learned from"));
    }
}
