use super::*;
use crate::interactive::tui::history::History;
use crate::interactive::tui::input::InputBuffer;
use crate::interactive::tui::transcript::Transcript;
use crate::interactive::tui::types::{App, OverlayState, RequestState};
use crate::interactive::tui::ui_snapshot_tests::{empty_app, unused_runtime, unused_store};
use crate::interactive::tui::ui_snapshot_tests::{fixed_status, render_buffer};
use std::path::PathBuf;

/// An idle app with the trust modal open, drawn through the real
/// `ui::draw`: the modal renders the shared prompt body and the
/// answers line beside the splash — the splash paints first, the
/// question rides inside the interface, never in front of it.
fn trust_app() -> App {
    App {
        sql_task: None,
        compact_task: None,
        input: InputBuffer::new(),
        transcript: Transcript::new(),
        profiles: vec!["analytics".into()],
        pending: None,
        request: RequestState::default(),
        overlays: OverlayState {
            trust: Some(TrustPrompt::default()),
            ..OverlayState::default()
        },
        spinner: 0,
        history: History::with_path_disabled(PathBuf::new()),
        viewport: std::cell::Cell::new((0, 0)),
        ctrl_c_armed: false,
        at_refs: Vec::new(),
        pending_clipboard: None,
        clipboard_copy: None,
        session_save: None,
        pending_session_save: None,
        last_query: None,
        wide_table: Default::default(),
        run_panel: None,
        runtime: unused_runtime(),
        state_db: unused_store(),
        session: std::sync::Arc::new(
            crate::interactive::session_universe::SessionUniverse::empty(),
        ),
        should_quit: false,
        pending_trust_answer: None,
    }
}

/// The modal renders the shared prompt body — the same words the plain
/// REPL prints — so the two renderings cannot drift. The TUI wraps the
/// body to the modal width and appends its own answers line on the
/// same visual row, so the assertion cuts the modal's words at the
/// body's own question mark: the words before it are the body's words
/// in order, wrapping aside.
#[test]
fn the_trust_modal_renders_the_shared_prompt_body() {
    let app = trust_app();
    let buffer = render_buffer(&app, &fixed_status(), 100, 30);
    let modal: String = buffer
        .lines()
        .filter(|line| line.contains('│'))
        .map(|line| line.replace(['│', '╭', '╰', '─', '"', ','], " "))
        .collect::<Vec<_>>()
        .join(" ");
    let body_end = modal.find("workspace?").expect("the body asks");
    let head = &modal[..body_end + "workspace?".len()];
    // The renderer strips the comma the source keeps after
    // "given" — punctuation the wrap drops, not a word the modal
    // rewords — so both sides drop commas before comparing.
    let flat: Vec<String> = head
        .split_whitespace()
        .map(|w| w.replace(',', ""))
        .collect();
    let body: Vec<String> = TRUST_PROMPT
        .split_whitespace()
        .take_while(|w| !w.starts_with("[t]rust"))
        .map(|w| w.replace(',', ""))
        .collect();
    let start = flat
        .iter()
        .position(|w| w == "No")
        .expect("the modal carries the body");
    assert_eq!(
        &flat[start..],
        body.as_slice(),
        "the modal renders the shared body words in order"
    );
}

/// The modal offers the same three exits the line prompt offers, and
/// the splash marker still paints behind it.
#[test]
fn the_trust_modal_offers_all_three_exits_beside_the_splash() {
    let app = trust_app();
    let buffer = render_buffer(&app, &fixed_status(), 100, 30);
    for exit in [
        "[t]rust once",
        "[w]orkspace <dir> instead",
        "[c]ontinue unbound",
    ] {
        assert!(
            buffer.contains(exit),
            "the modal offers every exit, missing {exit}\n{buffer}"
        );
    }
    assert!(
        buffer.contains("◆ saya"),
        "the splash paints behind the modal:\n{buffer}"
    );
}

/// A `w` draft with an inline refusal renders both the typed line and
/// the error — the refusal is said where the answer was typed.
#[test]
fn the_trust_modal_renders_the_draft_and_its_inline_refusal() {
    let mut app = trust_app();
    app.overlays.trust = Some(TrustPrompt {
        draft: Some("/definitely/not/here".into()),
        error: Some("the trusted folder /definitely/not/here could not be resolved".into()),
    });
    let buffer = render_buffer(&app, &fixed_status(), 100, 30);
    assert!(
        buffer.contains("w /definitely/not/here"),
        "the typed line renders:\n{buffer}"
    );
    assert!(
        buffer.contains("could not be resolved"),
        "the inline refusal renders:\n{buffer}"
    );
}

/// The modal's height accounts for the wrapped body plus the answers
/// and draft rows, bounded so a narrow terminal wraps instead of
/// pushing the input off screen.
#[test]
fn the_trust_modal_height_covers_body_answers_and_draft() {
    let plain = trust_modal_height(&TrustPrompt::default(), 100);
    let with_draft = trust_modal_height(
        &TrustPrompt {
            draft: Some("/tmp/proj".into()),
            error: None,
        },
        100,
    );
    assert!(
        with_draft > plain,
        "the draft claims rows: {plain} vs {with_draft}"
    );
    assert!(
        trust_modal_height(&TrustPrompt::default(), 100) <= 20,
        "the modal stays bounded"
    );
}

/// The empty-app helper stays honest: no test app opens the modal
/// unless it says so.
#[test]
fn empty_app_opens_no_trust_modal() {
    assert!(empty_app().overlays.trust.is_none());
}
