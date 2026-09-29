//! `submit()` is where a submitted line becomes persisted and displayed
//! copies (D2): history, the transcript's `User` block, and the queue notice
//! show the redacted form, while `pending` — what later dispatches — keeps
//! the original line so execution still receives the real values.

use super::*;
use crate::interactive::tui::application::tests_support::{idle_app, in_flight_task};
use crate::interactive::tui::history::History;

const VALUE: &str = "synthetic-confidential-customer-922";
const LINE: &str = "/investigation run inv-1 --param label=synthetic-confidential-customer-922";
const REDACTED: &str = "/investigation run inv-1 --param label=…";

fn tmp_path(tag: &str) -> std::path::PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    std::env::temp_dir().join(format!("saya_submit_redact_{tag}_{n}.txt"))
}

fn user_blocks(app: &crate::interactive::tui::types::App) -> Vec<&str> {
    app.transcript
        .blocks()
        .iter()
        .filter(|block| block.kind == BlockKind::User)
        .map(|block| block.text.as_str())
        .collect()
}

#[test]
fn the_user_block_and_history_show_the_redacted_line_but_pending_keeps_the_original() {
    let history_file = tmp_path("idle");
    let mut app = idle_app();
    app.history = History::with_path(history_file.clone());
    app.input.set_text(LINE);
    app.submit();

    assert_eq!(
        app.pending.as_deref(),
        Some(LINE),
        "execution keeps the original line with the real values"
    );
    assert_eq!(
        user_blocks(&app),
        vec![REDACTED],
        "the transcript's User block shows the redacted form"
    );
    assert_eq!(
        app.history.previous(),
        Some(REDACTED),
        "the history ring recalls the redacted form"
    );
    let file = std::fs::read_to_string(&history_file).unwrap();
    assert!(
        !file.contains(VALUE),
        "the value must not persist in the history file: {file:?}"
    );
    assert!(
        file.contains(REDACTED),
        "the file keeps the redacted form: {file:?}"
    );
    let _ = std::fs::remove_file(history_file);
}

#[test]
fn a_queued_run_quotes_the_redacted_form_in_the_notice() {
    let history_file = tmp_path("queued");
    let mut app = idle_app();
    app.history = History::with_path(history_file.clone());
    app.sql_task = Some(in_flight_task());
    assert!(app.is_busy(), "precondition: the app is busy");
    app.input.set_text(LINE);
    app.submit();

    assert_eq!(
        app.pending.as_deref(),
        Some(LINE),
        "the queued prompt still carries the original line for execution"
    );
    assert!(
        user_blocks(&app).is_empty(),
        "queueing adds no User block (existing rule)"
    );
    let notice = app
        .transcript
        .blocks()
        .iter()
        .find(|block| block.text.contains("Queued"))
        .expect("the queue notice exists");
    assert!(
        notice.text.contains(REDACTED),
        "the queued preview shows the redacted form: {:?}",
        notice.text
    );
    let transcript: String = app
        .transcript
        .blocks()
        .iter()
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !transcript.contains(VALUE),
        "the value must not echo anywhere in the transcript: {transcript:?}"
    );
    let _ = std::fs::remove_file(history_file);
}
