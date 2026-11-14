//! Report shaping for the portable context commands: the text one Result
//! message carries for `export` and `import`/`import-dbt`. Split from
//! `contracts_portable.rs` to keep both files small; JSON and NDJSON carry the
//! same message through the shared `Result` event, so machine consumers read
//! the same counts the terminal does.

use crate::contracts::portable::Unavailability;
use crate::contracts::portable::{ExportOutcome, ImportOutcome};

/// The dbt-specific part of an import report: the manifest version it came
/// from and the items the parser skipped with a stable reason.
pub(super) struct DbtProvenance<'a> {
    pub version: &'a str,
    pub skipped: &'a [(String, String)],
}

pub(super) fn export_report(outcome: &ExportOutcome) -> String {
    let mut lines = vec![format!(
        "exported {} claims to {}",
        outcome.total,
        outcome.path.display()
    )];
    for (kind, count) in &outcome.kinds {
        lines.push(format!("  {kind} {count}"));
    }
    if outcome.skipped > 0 {
        lines.push(format!(
            "  skipped {} claim(s) that could not be made portable",
            outcome.skipped
        ));
    }
    lines.push("Review the file before sharing.".to_string());
    lines.join("\n")
}

pub(super) fn import_report(
    profile_name: &str,
    schema_source: &str,
    outcome: &ImportOutcome,
    preview: bool,
    provenance: Option<&DbtProvenance<'_>>,
    memory_off: bool,
) -> String {
    let dbt_suffix = provenance
        .map(|provenance| format!("; dbt manifest {}", provenance.version))
        .unwrap_or_default();
    let unavailable = outcome.unavailable.len() + provenance.map(|p| p.skipped.len()).unwrap_or(0);
    let header = if preview {
        format!(
            "preview: would import {i}, skip {s}, conflict {c}, unavailable {u}  (profile: {p}, schema: {src}{dbt})",
            i = outcome.inserted.len(),
            s = outcome.skipped.len(),
            c = outcome.conflicts.len(),
            u = unavailable,
            p = profile_name,
            src = schema_source,
            dbt = dbt_suffix,
        )
    } else {
        format!(
            "imported {i}, skipped {s}, conflicts {c}, unavailable {u}  (profile: {p}, schema: {src}{dbt})",
            i = outcome.inserted.len(),
            s = outcome.skipped.len(),
            c = outcome.conflicts.len(),
            u = unavailable,
            p = profile_name,
            src = schema_source,
            dbt = dbt_suffix,
        )
    };
    let mut lines = vec![header];
    for (label, slot) in &outcome.inserted {
        lines.push(format!("  inserted  {label}  {slot}"));
    }
    for (label, slot) in &outcome.skipped {
        lines.push(format!("  skipped  {label}  {slot} — already present"));
    }
    for (label, slot, id) in &outcome.conflicts {
        lines.push(format!("  conflict  {label}  {slot} — existing claim {id}"));
    }
    for Unavailability { label, reason } in &outcome.unavailable {
        lines.push(format!("  unavailable  {label} — {reason}"));
    }
    for (label, reason) in provenance.map(|p| p.skipped).unwrap_or(&[]) {
        lines.push(format!("  unavailable  {label} — {reason}"));
    }
    lines.push(if preview {
        "Preview only: nothing was written.".to_string()
    } else {
        "Imported items are pending review: saya contracts queue".to_string()
    });
    if memory_off {
        lines.push("Memory is off, so these claims are not used until you enable it.".to_string());
    }
    lines.join("\n")
}
