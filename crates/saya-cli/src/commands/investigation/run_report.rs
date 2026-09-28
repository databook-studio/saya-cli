//! `saya investigation run --report` (S12b, D10): the headless replay writes
//! the same Markdown report the TUI's `/report` writes — one renderer, one
//! writer, no second implementation — from this replay's own result and
//! evidence. The report is written only after a successful execution and
//! after the query output is out, so a refused or failed write leaves the
//! destination untouched and exits 2 without disturbing the S7 binding
//! rules.

use super::run::RunRequest;
use crate::commands::output::{emit, failure_message};
use crate::interactive::tui::export::write_report;
use crate::render::{RenderFormat, TerminalEvent};
use crate::slash::MAX_REPORT_ROWS;
use saya_types::{ExecutionEvidence, QueryResult};

/// The usage refusals for the report flags, checked before anything runs
/// (invariant 1): `--rows` and `--overwrite` are report-only flags, and
/// `--rows` is capped at the shared [`MAX_REPORT_ROWS`] bound — the same
/// bound the slash parser and the renderer clamp to.
pub(super) fn usage_error(request: &RunRequest<'_>) -> Option<String> {
    if let Some(rows) = request.rows {
        if request.report.is_none() {
            return Some("--rows requires --report <path>".to_string());
        }
        if rows > MAX_REPORT_ROWS {
            return Some(format!("--rows is capped at {MAX_REPORT_ROWS}"));
        }
    }
    if request.overwrite && request.report.is_none() {
        return Some("--overwrite requires --report <path>".to_string());
    }
    None
}

/// Writes the report once the replay succeeded and its output is out
/// (invariant 3): a refused or failed write is an exit-2 error naming the
/// destination, and never a clobber of an existing file. On success the same
/// confirmation the TUI prints names what was written. Without `--report`
/// this is the plain success exit.
pub(super) fn write(
    result: &QueryResult,
    evidence: &ExecutionEvidence,
    request: &RunRequest<'_>,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    let Some(path) = request.report else {
        return Ok(0);
    };
    match write_report(result, evidence, request.rows, path, request.overwrite) {
        Ok(Some(included)) => {
            emit(
                TerminalEvent::Result {
                    message: format!(
                        "Wrote report to {} ({included} rows included)",
                        path.display()
                    ),
                },
                format,
            );
            Ok(0)
        }
        Ok(None) => {
            emit(
                TerminalEvent::Result {
                    message: format!("Wrote report to {} (rows omitted)", path.display()),
                },
                format,
            );
            Ok(0)
        }
        Err(message) => failure_message(super::EXIT_INVESTIGATION_ERROR, message, format),
    }
}
