//! The stop path: asking to stop, being told it stopped, and the
//! failure that must not be swallowed into either.
//!
//! Fixtures are shared with `guard_tests` rather than duplicated: two copies
//! of a stream builder drift, and these assert on the same drain path.

use super::guard_tests::{app_with_model, cancelled_stream_with, stream_with};
use super::*;

/// Phase 5 packet 1: the worker's stop confirmation reads as a stop, not a
/// failure — "Stopped." with kept work — and is not an error block.
#[test]
fn the_worker_confirmation_says_stopped() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    app.request.stream = Some(cancelled_stream_with(vec![StreamMsg::Done(Err(
        "request cancelled".into(),
    ))]));
    assert!(app.drain_stream(&mut state), "the turn finished");
    let last = app
        .transcript
        .blocks()
        .last()
        .expect("the confirmation was pushed");
    assert!(
        last.text.contains("Stopped."),
        "the worker confirmation must say Stopped.: {}",
        last.text
    );
}

/// Phase 5 packet 1: a user stop is not rendered as an error — the
/// confirmation is a System block, without the failure mark.
#[test]
fn a_user_stop_is_not_rendered_as_an_error() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    app.request.stream = Some(cancelled_stream_with(vec![StreamMsg::Done(Err(
        "request cancelled".into(),
    ))]));
    assert!(app.drain_stream(&mut state), "the turn finished");
    assert!(
        !app.transcript
            .blocks()
            .iter()
            .any(|block| block.kind == BlockKind::Error),
        "a user stop must not push an error block"
    );
}

/// Phase 5 packet 1: a genuine failure is still an error — the fix must not
/// swallow real failures into the stop wording.
#[test]
fn a_genuine_failure_is_still_an_error() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    app.request.stream = Some(stream_with(vec![StreamMsg::Done(Err(
        "connection refused".into(),
    ))]));
    assert!(app.drain_stream(&mut state), "the turn finished");
    let last = app
        .transcript
        .blocks()
        .last()
        .expect("the failure was pushed");
    assert_eq!(
        last.kind,
        BlockKind::Error,
        "a genuine failure must stay an error block"
    );
}

/// Phase 5 packet 1: stopping never claims work was undone — no rollback
/// language on either the request or the confirmation.
#[test]
fn stopping_never_claims_work_was_undone() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    app.request.stream = Some(cancelled_stream_with(vec![StreamMsg::Done(Err(
        "request cancelled".into(),
    ))]));
    assert!(app.drain_stream(&mut state), "the turn finished");
    for block in app.transcript.blocks() {
        let lower = block.text.to_lowercase();
        for word in ["undone", "reverted", "rolled back"] {
            assert!(
                !lower.contains(word),
                "stop wording must never imply rollback ({word}): {}",
                block.text
            );
        }
    }
}

/// The dangerous case the `&&` exists for: the user presses Esc, and while
/// the stop is in flight the connection genuinely drops. Guarding on the
/// fired token alone would swallow that failure into a reassuring
/// "Stopped. Completed work is kept." — telling the user their work ended the
/// way they asked when in fact it broke.
#[test]
fn a_failure_arriving_after_esc_is_still_an_error() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    let stream = stream_with(vec![StreamMsg::Done(Err("connection refused".into()))]);
    stream.cancel.cancel();
    app.request.stream = Some(stream);
    assert!(app.drain_stream(&mut state), "the turn finished");
    let last = app
        .transcript
        .blocks()
        .last()
        .expect("the failure was pushed");
    assert_eq!(
        last.kind,
        BlockKind::Error,
        "a real failure during a stop is still a failure, not a clean stop"
    );
    assert!(
        last.text.contains("connection refused"),
        "and it still names what went wrong: {}",
        last.text
    );
}
