use super::super::transcript::BlockKind;
/// Composed-screen behaviour snapshots (submit-time fold and composer hints), moved byte-identical from the hub.
/// Plain assertions only — the five `insta` snapshots stay in the hub.
use super::support::{empty_app, fixed_status, render_buffer};

// --- Fieldnotes phase 3, packet 3: automatic fold at the live edge. ---------
//
// When the user sends a new request, the chapter that just finished folds
// itself before the new `User` block lands. Pin the observable screen: the
// finished chapter's answer leaves the buffer while its verbatim request line
// stays, without blessing a new snapshot (the fold path is already snapshotted
// above; this asserts the submit-time trigger paints the same screen).
#[test]
fn sending_a_new_request_folds_the_finished_chapter_on_screen() {
    use super::super::application::tests_support::idle_app;
    let mut app = idle_app();
    app.input.set_text("count the red orders");
    app.submit();
    app.pending = None;
    app.transcript
        .push(BlockKind::Assistant, "the red orders total 42");
    app.input.set_text("and the blue ones");
    app.submit();
    app.pending = None;
    assert!(
        app.transcript.is_folded(1),
        "the finished chapter folds when the next request starts"
    );
    let screen = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        screen.contains("count the red orders"),
        "the folded screen keeps the verbatim request:\n{screen}"
    );
    assert!(
        !screen.contains("the red orders total 42"),
        "the hidden answer leaves the screen:\n{screen}"
    );
}

// --- Fieldnotes phase 2, packet 2C: the composer says what Send will do. ----
//
// The composer placeholder only paints while the input is empty; the moment
// the user types, nothing on screen says what Enter does. These tests pin the
// packet's one observable outcome: the composer always states what Enter will
// do, with a different hint when the draft is multiline.
const ENTER_SENDS_HINT: &str = "Enter sends";
const MULTILINE_SENDS_HINT: &str = "Enter sends all";
const NEWLINE_HINT: &str = "Alt+Enter";

/// With a single-line draft, the rendered frame contains the send hint. Uses
/// `render_buffer` (colour-stripped), so this is hue-independent.
#[test]
fn the_composer_says_what_enter_does_while_typing() {
    let mut app = empty_app();
    app.input.set_text("how many orders yesterday");
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        buffer.contains(ENTER_SENDS_HINT),
        "a single-line draft states that Enter sends:\n{buffer}"
    );
    assert!(
        !buffer.contains(NEWLINE_HINT),
        "a single-line draft does not name the newline chord:\n{buffer}"
    );
}

/// With a 3-line draft, the hint differs from the single-line one: it states
/// Enter sends every line and names Alt+Enter as the way to add another line.
#[test]
fn a_multiline_draft_warns_that_enter_sends_every_line() {
    let mut app = empty_app();
    app.input.set_text("select *\nfrom orders\nlimit 10");
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        buffer.contains(MULTILINE_SENDS_HINT),
        "a multiline draft warns Enter sends every line:\n{buffer}"
    );
    assert!(
        buffer.contains(NEWLINE_HINT),
        "a multiline draft names Alt+Enter for a new line:\n{buffer}"
    );
}

/// Bracketed paste already lands as one event and stays editable: a pasted
/// 3-line block holds all three lines in the draft and submits nothing.
#[test]
fn a_pasted_block_stays_editable_instead_of_submitting() {
    let mut app = empty_app();
    app.paste("line one\nline two\nline three");
    assert_eq!(
        app.input.lines(),
        vec!["line one", "line two", "line three"],
        "the pasted block holds all three lines as an editable draft"
    );
    assert!(app.pending.is_none(), "pasting must not submit anything");
    assert!(
        app.transcript.blocks().is_empty(),
        "pasting must not append to the transcript"
    );
}

/// The draft buffer survives opening and closing an overlay: set a draft,
/// open the help overlay, close it, and the draft is intact.
#[test]
fn the_draft_survives_opening_and_closing_an_overlay() {
    use super::super::keys::handle_key;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    let mut app = empty_app();
    app.input.set_text("select * from orders");
    handle_key(&mut app, KeyCode::F(1), KeyModifiers::NONE);
    assert!(
        app.overlays.show_help,
        "F1 opens the help overlay while a draft is held"
    );
    // Any key dismisses the help overlay; the draft must survive the round trip.
    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(!app.overlays.show_help, "Esc closes the help overlay");
    assert_eq!(
        app.input.text(),
        "select * from orders",
        "the draft survives opening and closing the overlay"
    );
}
