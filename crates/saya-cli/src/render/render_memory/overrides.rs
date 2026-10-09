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
