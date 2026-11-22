//! `/export`'s two writers, chosen by the one parsed request (`03` §4.1).
//!
//! A `.md` destination — or `--format md` — writes today's Markdown report
//! from the result the surface holds and the execution it came from; a data
//! destination goes to today's data writer, told the format the request
//! states. The line REPL's dispatch makes the same choice over its capture,
//! through the same two calls, so both write the same bytes for the same
//! result.
//!
//! Neither writer queries a database: the rows are the ones the surface
//! already shows.

use std::path::Path;

use saya_types::{ExecutionEvidence, QueryResult};

use super::direct::DirectOutcome;
use super::msg::Worker;
use crate::interactive::tui::export::write_report;
use crate::interactive::tui::write_result_for;
use crate::slash::ExportRequest;

/// What the surface says when a result has no execution to report: the
/// provenance is what the document is made of.
pub(crate) const NO_PROVENANCE: &str = "that result carries no execution to report.";

/// One `/export`, to the path its request names.
pub(crate) fn export(
    result: &QueryResult,
    evidence: Option<&ExecutionEvidence>,
    request: &ExportRequest,
) -> Worker {
    if request.is_report() {
        return report(result, evidence, request);
    }
    match write_result_for(result, request) {
        Ok(rows) => Worker {
            message: format!("Exported {rows} row(s) to {}", request.path),
            ..Worker::default()
        },
        Err(message) => Worker {
            direct: Some(DirectOutcome::Error(message)),
            ..Worker::default()
        },
    }
}

/// The report arm: today's `write_report`, over the result the surface holds
/// and the execution it came from — the same call, over the same facts, the
/// line REPL's `/export out.md` makes from its capture.
fn report(
    result: &QueryResult,
    evidence: Option<&ExecutionEvidence>,
    request: &ExportRequest,
) -> Worker {
    let Some(evidence) = evidence else {
        return Worker {
            direct: Some(DirectOutcome::Error(NO_PROVENANCE.to_owned())),
            ..Worker::default()
        };
    };
    match write_report(
        result,
        evidence,
        request.rows,
        Path::new(&request.path),
        request.overwrite,
    ) {
        Ok(rows) => {
            let included = match rows {
                Some(rows) => format!("{rows} rows included"),
                None => "rows omitted".to_owned(),
            };
            Worker {
                message: format!("Wrote report to {} ({included})", request.path),
                ..Worker::default()
            }
        }
        Err(message) => Worker {
            direct: Some(DirectOutcome::Error(message)),
            ..Worker::default()
        },
    }
}
