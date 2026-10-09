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
                // the recall path points at the same next step the learn
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
/// displayed it, without finding and copying a 64-character id.
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
