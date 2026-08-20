//! Text shaping for contract import/export reports (slice 6b). JSON and NDJSON
//! fall out of the serde derives on [`TerminalEvent`](super::TerminalEvent); the
//! text form needs deliberate shaping, done here. Mirrors
//! [`render_contract`](super::render_contract).

use super::{ContractExportView, ContractImportView, Rendered};

/// The import report: a header naming the profile and dry-run posture, one line
/// per claim with its verdict, then any rejected files and a truncation note.
/// Empty verdict buckets render no lines, so a clean dry run reads plainly.
pub(super) fn import(report: &ContractImportView) -> Rendered {
    let mut stdout = String::new();
    let posture = if report.dry_run {
        "dry run"
    } else {
        "imported"
    };
    stdout.push_str(&format!(
        "{posture} {added} added, {dup} duplicate, {conf} conflicting, {stale} stale  (profile: {profile})\n",
        added = count(report, "added"),
        dup = count(report, "duplicate"),
        conf = count(report, "conflicting"),
        stale = count(report, "stale"),
        profile = report.profile,
    ));
    for claim in &report.claims {
        stdout.push_str(&format!(
            "  {verdict}  {object}  {source}{detail}\n",
            verdict = claim.verdict,
            object = claim.object,
            source = claim.source,
            detail = verdict_detail(claim),
        ));
    }
    for (path, reason) in &report.rejected {
        stdout.push_str(&format!("  rejected  {path}  {reason}\n"));
    }
    if let Some(bound) = &report.truncated_by {
        stdout.push_str(&format!("  truncated by {bound}\n"));
    }
    Rendered {
        stdout,
        stderr: String::new(),
    }
}

/// The export report: the files written and a count of skipped Relationship
/// claims, named so the skip is reported rather than silent.
pub(super) fn export(report: &ContractExportView) -> Rendered {
    let mut stdout = String::new();
    stdout.push_str(&format!(
        "exported {n} file(s)  (profile: {profile})\n",
        n = report.written.len(),
        profile = report.profile,
    ));
    for path in &report.written {
        stdout.push_str(&format!("  wrote {path}\n"));
    }
    if report.skipped_relationship > 0 {
        stdout.push_str(&format!(
            "  skipped {n} relationship claim(s) — not representable in the v1 file shape\n",
            n = report.skipped_relationship,
        ));
    }
    Rendered {
        stdout,
        stderr: String::new(),
    }
}

fn count(report: &ContractImportView, verdict: &str) -> usize {
    report
        .claims
        .iter()
        .filter(|c| c.verdict == verdict)
        .count()
}

fn verdict_detail(claim: &super::ContractImportClaimView) -> String {
    match claim.verdict.as_str() {
        "duplicate" => claim
            .existing_status
            .as_deref()
            .map(|s| format!("  ({s})"))
            .unwrap_or_default(),
        "conflicting" => claim
            .existing_id
            .as_deref()
            .map(|id| format!("  (existing {id})"))
            .unwrap_or_default(),
        _ => String::new(),
    }
}
