//! Markdown report rendering (S12): `/report` turns the captured `/sql`
//! result into a shareable Markdown file. The builder is pure — all I/O
//! lives in `write_report` (`super`). Default content is the SQL and
//! provenance only; rows land only on `--rows N`, neutralised, under the
//! 2 MiB ceiling.

use super::markdown_text::{neutralize_cell, neutralize_text, sql_block, utc_timestamp};
use super::shared::{BoundedWriter, normalize_row};
use crate::slash::MAX_REPORT_ROWS;
use saya_types::{ExecutionEvidence, QueryResult, ResultScope};

/// The hard ceiling on one rendered report, checked while building.
pub(crate) const MAX_REPORT_BYTES: usize = 2 * 1024 * 1024;
const REPORT_TOO_LARGE: &str = "report larger than 2 MiB; include fewer rows";

/// Everything the report is built from. `generated_unix_ms` is the wall
/// clock at write time, supplied by the caller so rendering stays pure.
pub(crate) struct ReportInput<'a> {
    pub(crate) result: &'a QueryResult,
    pub(crate) evidence: &'a ExecutionEvidence,
    /// Rows requested for the table; `None` omits rows.
    pub(crate) rows: Option<usize>,
    pub(crate) generated_unix_ms: i64,
}

/// Renders the report document, refusing anything over [`MAX_REPORT_BYTES`].
pub(crate) fn render_report(input: &ReportInput<'_>) -> Result<String, String> {
    render_report_with(input, MAX_REPORT_BYTES)
}

/// Test seam: a smaller ceiling exercises the while-building stop.
#[cfg(test)]
pub(crate) fn render_report_with_ceiling(
    input: &ReportInput<'_>,
    ceiling: usize,
) -> Result<String, String> {
    render_report_with(input, ceiling)
}

/// The rows the table includes: clamped to the shared [`MAX_REPORT_ROWS`]
/// bound (the parser refuses above it) and the captured row count.
pub(crate) fn clamped_rows(rows: Option<usize>, result: &QueryResult) -> Option<usize> {
    rows.map(|n| n.min(MAX_REPORT_ROWS).min(result.rows.len()))
}

/// One bounded write; any refusal is the size ceiling in the report's words.
fn section(out: &mut BoundedWriter, text: &str) -> Result<(), String> {
    out.write_str(text)
        .map_err(|_| REPORT_TOO_LARGE.to_string())
}

fn render_report_with(input: &ReportInput<'_>, ceiling: usize) -> Result<String, String> {
    let mut out = BoundedWriter::new(ceiling);
    let (result, evidence) = (input.result, input.evidence);
    section(&mut out, "# saya report\n\n")?;
    let generated = format!(
        "Generated {} by saya {}\n\n",
        utc_timestamp(input.generated_unix_ms),
        env!("CARGO_PKG_VERSION")
    );
    section(&mut out, &generated)?;
    section(&mut out, "## Query\n\n")?;
    section(&mut out, &sql_block(&result.executed_sql))?;
    section(&mut out, "## Provenance\n\n")?;
    for bullet in provenance_bullets(evidence) {
        section(&mut out, &format!("- {bullet}\n"))?;
    }
    section(&mut out, "\n## Rows\n\n")?;
    match clamped_rows(input.rows, result) {
        Some(included) => {
            render_rows(&mut out, result, included)?;
            let total = result.rows.len();
            section(
                &mut out,
                &format!("\nShowing {included} of {total} captured rows.\n"),
            )?;
            if evidence.truncated {
                let cap = evidence.max_rows;
                section(
                    &mut out,
                    &format!("The query result itself was truncated at {cap} rows.\n"),
                )?;
            }
        }
        None => section(
            &mut out,
            "Rows omitted (pass --rows N to include up to 100).\n",
        )?,
    }
    Ok(String::from_utf8(out.into_inner()).expect("report bytes are UTF-8"))
}

/// The provenance bullets, exactly: connection label, submitted-SQL hash,
/// execution id, start/finish (UTC), row counts, truncation, scope.
fn provenance_bullets(evidence: &ExecutionEvidence) -> Vec<String> {
    let scope = match &evidence.scope {
        ResultScope::Full => "full result".to_owned(),
        ResultScope::ModelLimited { row_cap } => format!("model-limited (first {row_cap} rows)"),
    };
    vec![
        format!(
            "Connection label: {}",
            neutralize_text(&evidence.connection_label)
        ),
        format!("Submitted SQL sha256: {}", evidence.submitted_sql_sha256),
        format!("Execution id: {}", evidence.short_id()),
        format!("Started: {}", utc_timestamp(evidence.started_unix_ms)),
        format!("Finished: {}", utc_timestamp(evidence.finished_unix_ms)),
        format!("Returned rows: {}", evidence.returned_rows),
        format!("Row cap: {}", evidence.max_rows),
        format!(
            "Truncated: {}",
            if evidence.truncated { "yes" } else { "no" }
        ),
        format!("Scope: {scope}"),
    ]
}

fn render_rows(
    out: &mut BoundedWriter,
    result: &QueryResult,
    included: usize,
) -> Result<(), String> {
    let header = result
        .columns
        .iter()
        .map(|column| neutralize_text(column))
        .collect::<Vec<_>>()
        .join(" | ");
    section(
        out,
        &format!(
            "| {header} |\n|{}\n",
            " --- |".repeat(result.columns.len().max(1))
        ),
    )?;
    for row in result.rows.iter().take(included) {
        let cells = normalize_row(row, result.columns.len())
            .iter()
            .map(neutralize_cell)
            .collect::<Vec<_>>()
            .join(" | ");
        section(out, &format!("| {cells} |\n"))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "markdown_tests.rs"]
mod tests;
