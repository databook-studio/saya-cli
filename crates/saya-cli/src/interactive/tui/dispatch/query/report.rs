//! The `/report` arm of the query dispatcher: the Markdown report is written
//! from the captured result, with no query and no task.

use crate::interactive::tui::capture::CapturedResult;
use crate::interactive::tui::export::write_report;
use crate::interactive::tui::transcript::{BlockKind, Transcript};
use saya_types::{ExecutionEvidence, ResultScope};

/// Writes the Markdown report from the captured result: no query, no task —
/// the report is the capture, plus provenance from its evidence. Nothing
/// here queries a database.
pub(super) fn apply_report(
    request: &crate::slash::ReportRequest,
    transcript: &mut Transcript,
    captured: &Option<CapturedResult>,
) {
    let Some(capture) = captured else {
        transcript.push(
            BlockKind::Error,
            "No captured result to report. Run the query with /sql first; captures \
             last only for this session.",
        );
        return;
    };
    let path = std::path::Path::new(&request.path);
    match write_report(
        &capture.result,
        &capture.evidence,
        request.rows,
        path,
        request.overwrite,
    ) {
        Ok(Some(n)) => transcript.push(
            BlockKind::System,
            format!(
                "Wrote report to {} ({n} rows included{})",
                request.path,
                scope_note(&capture.evidence)
            ),
        ),
        Ok(None) => transcript.push(
            BlockKind::System,
            format!(
                "Wrote report to {} (rows omitted{})",
                request.path,
                scope_note(&capture.evidence)
            ),
        ),
        Err(msg) => transcript.push(BlockKind::Error, msg),
    }
}

/// The scope label every report success message carries (D12): a
/// model-limited capture says what the rows are and that the full result may
/// be larger.
fn scope_note(evidence: &ExecutionEvidence) -> String {
    match &evidence.scope {
        ResultScope::Full => String::new(),
        ResultScope::ModelLimited { row_cap } => format!(
            "; model-limited: the agent saw only the first {row_cap} rows — \
             the full result may be larger"
        ),
    }
}
