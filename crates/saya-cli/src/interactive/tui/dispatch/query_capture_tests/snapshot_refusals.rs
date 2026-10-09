//! The snapshot refusal when the latest promoted agent query's rows are not
//! held: it names why — the budget, a missing capture, a truncated or redacted
//! model view — and never the /sql-only wording.

use super::{apply_query_actions, last_block, snapshot_request};
use crate::interactive::session::commands::SessionAction;
use crate::interactive::session::state::SessionState;
use crate::interactive::tui::application::tests_support::unused_runtime;
use crate::interactive::tui::capture_agent::CaptureGap;
use crate::interactive::tui::transcript::{BlockKind, Transcript};

/// With nothing captured right after a promoted agent query, the refusal
/// names the gap — the budget when the capture was refused, nothing when no
/// capture arrived — and never the /sql-only wording. With no gap at all the
/// established wording stands.
#[test]
fn snapshot_after_uncaptured_agent_query_names_the_gap() {
    let mut transcript = Transcript::new();
    let mut state = SessionState::new("test", None, "model");
    let rt = unused_runtime();
    let request = SessionAction::Export(snapshot_request(std::path::Path::new(
        "/nonexistent-saya-c2/never.csv",
    )));
    apply_query_actions(
        &mut Some(request.clone()),
        &mut transcript,
        &mut state,
        &mut None,
        &None,
        Some(CaptureGap::OverBudget),
        &rt,
    );
    let msg = last_block(&transcript, BlockKind::Error).expect("the refusal is an error");
    assert_eq!(
        msg,
        "The latest query's rows were not captured (larger than 32 MiB). \
         Use /export --refresh to re-run it.",
        "the refusal names the budget: {msg}"
    );
    apply_query_actions(
        &mut Some(request.clone()),
        &mut transcript,
        &mut state,
        &mut None,
        &None,
        Some(CaptureGap::Missing),
        &rt,
    );
    let msg = last_block(&transcript, BlockKind::Error).expect("the refusal is an error");
    assert_eq!(
        msg, "The latest query's rows were not captured. Use /export --refresh to re-run it.",
        "no reason is invented for a missing capture: {msg}"
    );
    apply_query_actions(
        &mut Some(request),
        &mut transcript,
        &mut state,
        &mut None,
        &None,
        None,
        &rt,
    );
    let msg = last_block(&transcript, BlockKind::Error).expect("the refusal is an error");
    assert!(
        msg.contains("No captured result to snapshot"),
        "with no gap the established wording stands: {msg}"
    );
}

/// R3 decision 3: when the promoted agent query's model view was truncated,
/// the snapshot refusal names that reason — the exact sentence, with the
/// refresh way out.
#[test]
fn snapshot_after_truncated_model_view_names_the_reason() {
    let mut transcript = Transcript::new();
    let mut state = SessionState::new("test", None, "model");
    let rt = unused_runtime();
    apply_query_actions(
        &mut Some(SessionAction::Export(snapshot_request(
            std::path::Path::new("/nonexistent-saya-r3/never.csv"),
        ))),
        &mut transcript,
        &mut state,
        &mut None,
        &None,
        Some(CaptureGap::ModelViewTruncated),
        &rt,
    );
    let msg = last_block(&transcript, BlockKind::Error).expect("the refusal is an error");
    assert_eq!(
        msg,
        "The agent received a truncated version of this result, so it was \
         not captured. Use /export --refresh to re-run it.",
        "the refusal names the truncated model view: {msg}"
    );
}

/// R3 decision 3: the redacted model view gets the same refusal, naming
/// redaction — the model never saw the raw values, so they are not evidence.
#[test]
fn snapshot_after_redacted_model_view_names_the_reason() {
    let mut transcript = Transcript::new();
    let mut state = SessionState::new("test", None, "model");
    let rt = unused_runtime();
    apply_query_actions(
        &mut Some(SessionAction::Export(snapshot_request(
            std::path::Path::new("/nonexistent-saya-r3/never.csv"),
        ))),
        &mut transcript,
        &mut state,
        &mut None,
        &None,
        Some(CaptureGap::ModelViewRedacted),
        &rt,
    );
    let msg = last_block(&transcript, BlockKind::Error).expect("the refusal is an error");
    assert_eq!(
        msg,
        "The agent received a redacted version of this result, so it was \
         not captured. Use /export --refresh to re-run it.",
        "the refusal names the redacted model view: {msg}"
    );
}
