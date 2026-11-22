//! The Markdown report arm of the query dispatcher: `/export` with a `.md`
//! destination (or `--format md`) — the form `/report` spells — writes the
//! document from the captured result, with no query and no task.

use crate::chat::ops_status::Note;
use crate::interactive::session::state::SessionState;
use crate::interactive::tui::capture::CapturedResult;
use crate::interactive::tui::export::write_report;
use crate::interactive::tui::transcript::{BlockKind, Transcript};
use crate::slash::ExportRequest;
use saya_types::{ExecutionEvidence, ResultScope};

/// Writes the Markdown report from the captured result: no query, no task —
/// the report is the capture, plus provenance from its evidence. Nothing
/// here queries a database. `/report` is this request's one-release alias, so
/// its note rides the same message, once per session.
pub(super) fn apply_report(
    request: &ExportRequest,
    transcript: &mut Transcript,
    state: &mut SessionState,
    captured: &Option<CapturedResult>,
) {
    let Some(capture) = captured else {
        transcript.push(
            BlockKind::Error,
            "No captured result to report. Run the query with /sql first; captures \
             last only for this session."
                .to_string(),
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
            with_alias_note(
                state,
                request,
                format!(
                    "Wrote report to {} ({n} rows included{})",
                    request.path,
                    scope_note(&capture.evidence)
                ),
            ),
        ),
        Ok(None) => transcript.push(
            BlockKind::System,
            with_alias_note(
                state,
                request,
                format!(
                    "Wrote report to {} (rows omitted{})",
                    request.path,
                    scope_note(&capture.evidence)
                ),
            ),
        ),
        Err(msg) => transcript.push(BlockKind::Error, msg),
    }
}

/// The alias's note, said the first time this session reports: `/report` is
/// `/export --format md`.
fn with_alias_note(state: &mut SessionState, request: &ExportRequest, text: String) -> String {
    if request.via_report_alias {
        crate::chat::ops_status::with_note(state, Note::Report, text)
    } else {
        text
    }
}

/// The scope label every report success message carries: a
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
