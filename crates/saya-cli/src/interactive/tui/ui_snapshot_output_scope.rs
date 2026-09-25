//! Output-scope wording: what each copy key and query follow-up acts on.

use super::super::transcript::BlockKind;
use super::{empty_app, fixed_status, render_buffer};

/// Ctrl+Y copies the last assistant block; Ctrl+B copies the full transcript
/// minus thinking. No help rewording may change either path.
#[test]
fn copy_behaviour_is_unchanged() {
    // Ctrl+Y: the last assistant block, even with thinking on screen.
    let mut app = empty_app();
    app.transcript.push(BlockKind::User, "show the orders");
    app.transcript
        .push(BlockKind::Assistant, "the answer is 42");
    app.transcript
        .push(BlockKind::Thinking, "the secret chain-of-thought");
    app.copy_last_answer();
    assert_eq!(
        app.pending_clipboard.as_deref(),
        Some("the answer is 42"),
        "Ctrl+Y still yields the last assistant block"
    );

    // Ctrl+B: the full untruncated transcript minus thinking.
    let mut app = empty_app();
    app.transcript.push(BlockKind::User, "what is the answer");
    app.transcript
        .push(BlockKind::Thinking, "the secret chain-of-thought");
    app.transcript
        .push(BlockKind::Assistant, "the answer is 42");
    app.copy_transcript();
    let copied = app.pending_clipboard.expect("transcript was queued");
    assert!(
        copied.contains("what is the answer") && copied.contains("the answer is 42"),
        "Ctrl+B still yields the full transcript: {copied}"
    );
    assert!(
        !copied.contains("the secret chain-of-thought"),
        "Ctrl+B still leaves thinking out: {copied}"
    );
}

/// The keybinding help overlay names what each copy key acts on: Ctrl+Y the
/// last assistant block (not a table), Ctrl+B the full transcript minus
/// reasoning.
#[test]
fn help_overlay_names_what_each_copy_key_acts_on() {
    let mut app = empty_app();
    app.overlays.show_help = true;
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        buffer.contains("last assistant"),
        "the overlay must say Ctrl+Y copies the last assistant block:\n{buffer}"
    );
    assert!(
        buffer.contains("minus reasoning") || buffer.contains("minus thinking"),
        "the overlay must say Ctrl+B leaves reasoning out:\n{buffer}"
    );
    insta::assert_snapshot!(buffer);
}
