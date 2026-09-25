//! The approval panel's answers stay painted however long the fact body is:
//! a body that outruns the capped panel used to push the answers line out of
//! the panel while the keys still worked — a guessed keypress still consented
//! (audit finding F05). These tests paint real composed frames through the
//! real `ui::draw` onto a `TestBackend` and assert on the buffer.

use crate::interactive::tui::types::{App, PendingApproval};
use crate::interactive::tui::ui_snapshot_tests::{empty_app, fixed_status, render_buffer};

/// The audit's repro: a detail body whose wrapped rows outrun the capped
/// panel, so the answers line — last in the flat paragraph — is the first
/// thing clipped.
fn long_detail() -> String {
    (1..=30)
        .map(|i| format!("detail line {i:02} — a consequence the user must weigh"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// An idle app with a pending approval carrying `detail` and `grant`.
fn approval_app(detail: Option<String>, grant: Option<&str>) -> App {
    let mut app = empty_app();
    let (respond, _answer) = tokio::sync::oneshot::channel();
    app.request.pending_approval = Some(PendingApproval {
        tool: "http_fetch".into(),
        detail,
        grant: grant.map(str::to_string),
        scroll: 0,
        respond,
    });
    app
}

/// However long the detail body is, the answers line is painted inside the
/// panel: the answers are reserved out of the panel height, not appended
/// after a body that eats the room.
#[test]
fn a_long_detail_still_shows_the_answers() {
    let app = approval_app(Some(long_detail()), Some("fetch:https+api.github.com"));
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        buffer.contains("[a] allow once"),
        "the answers line survives a long detail body:\n{buffer}"
    );
    assert!(
        buffer.contains("[s] allow fetch:https+api.github.com"),
        "the session answer survives a long detail body:\n{buffer}"
    );
}

/// A clipped body is labelled: the user must be able to tell material was
/// withheld — a partial consequence shown silently is the permission defect.
#[test]
fn a_long_detail_says_it_was_truncated() {
    let app = approval_app(Some(long_detail()), Some("fetch:https+api.github.com"));
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        buffer.contains("more lines hidden"),
        "the overflow is labelled on the last visible detail row:\n{buffer}"
    );
}

/// The control: when every fact fits, all of them paint with the answers and
/// nothing is marked. A fix that always truncates fails here.
#[test]
fn a_short_detail_paints_every_fact_and_the_answers() {
    let detail = "first fact\nsecond fact\nthird fact".to_string();
    let app = approval_app(Some(detail), Some("fetch:https+api.github.com"));
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    for fact in ["first fact", "second fact", "third fact"] {
        assert!(
            buffer.contains(fact),
            "every fact paints when the body fits: missing {fact}\n{buffer}"
        );
    }
    assert!(
        buffer.contains("[a] allow once"),
        "the answers paint when the body fits:\n{buffer}"
    );
    assert!(
        !buffer.contains("more lines hidden"),
        "nothing is marked when nothing is clipped:\n{buffer}"
    );
}

/// A narrow frame wraps the answers into more rows and still paints them —
/// the reservation follows the wrapped answers, not the panel floor.
#[test]
fn a_narrow_frame_still_shows_the_answers() {
    let app = approval_app(Some(long_detail()), Some("fetch:https+api.github.com"));
    let buffer = render_buffer(&app, &fixed_status(), 60, 24);
    assert!(
        buffer.contains("[a] allow once"),
        "the answers line survives a long body at 60 columns:\n{buffer}"
    );
    assert!(
        buffer.contains("[s] allow fetch:https+api.github.com"),
        "the session answer survives a long body at 60 columns:\n{buffer}"
    );
}
