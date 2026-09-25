//! The draft label must survive an overflowing pane (audit finding F04):
//! exactly the live answer is marked `SAYA (draft)` in conversations longer
//! than the visible pane — and every older answer still reads as final. These
//! render real frames through the composed `ui::draw` onto a ratatui test
//! backend and assert on the painted buffer, because the defect only shows
//! when the painted window's first row is not the transcript's first row.

use crate::interactive::tui::stream_events::apply_event;
use crate::interactive::tui::transcript::BlockKind;
use crate::interactive::tui::types::App;
use crate::interactive::tui::ui_snapshot_tests::{empty_app, fixed_status, render_buffer};
use saya_agent::AgentEvent;

/// A streaming request whose channel never delivers: `request.stream.is_some()`
/// reads true so the draft path renders, with no provider behind it.
fn busy_stream() -> crate::interactive::tui::agent::Stream {
    let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
    crate::interactive::tui::agent::Stream {
        rx,
        cancel: saya_agent::CancellationToken::new(),
        prompt: String::new(),
    }
}

/// Four finished turns — long enough that the transcript overflows an 80×24
/// pane — then the live turn streaming in at the tail.
fn overflowing_app() -> App {
    let mut app = empty_app();
    for turn in 1..=4 {
        app.transcript
            .push(BlockKind::User, format!("question {turn}"));
        apply_event(
            &mut app.transcript,
            AgentEvent::assistant_text(format!(
                "answer {turn} part a\nanswer {turn} part b\nanswer {turn} part c"
            )),
            false,
        );
    }
    app.transcript.push(BlockKind::User, "question five");
    apply_event(
        &mut app.transcript,
        AgentEvent::assistant_text("arriving part a\narriving part b\narriving part c"),
        false,
    );
    app.request.stream = Some(busy_stream());
    app
}

/// Screen rows as the test backend paints them, one entry per row.
fn painted_lines(buffer: &str) -> Vec<&str> {
    buffer.lines().collect()
}

/// The packet's core test: a conversation comfortably longer than an 80×24
/// pane, a stream in flight, the live answer visible at the bottom — its
/// label must read `SAYA (draft)`, not a final-looking `SAYA`.
#[test]
fn the_live_answer_is_marked_draft_when_the_pane_overflows() {
    let app = overflowing_app();
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        buffer.contains("arriving part a"),
        "precondition: the live answer itself is on screen:\n{buffer}"
    );
    assert!(
        buffer.contains("SAYA (draft)"),
        "the live answer's label must read as a draft past one screen:\n{buffer}"
    );
}

/// Same overflowing transcript: exactly one draft marker, and it sits on the
/// last answer — every older `SAYA` stays final.
#[test]
fn an_older_answer_is_never_marked_draft() {
    let app = overflowing_app();
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    let lines = painted_lines(&buffer);
    let markers: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter_map(|(i, line)| line.contains("SAYA (draft)").then_some(i))
        .collect();
    assert_eq!(
        markers.len(),
        1,
        "exactly one draft marker may paint:\n{buffer}"
    );
    let below = lines.get(markers[0] + 1).copied().unwrap_or("");
    assert!(
        below.contains("arriving part a"),
        "the marker must sit on the live answer, not an older one:\n{buffer}"
    );
}

/// Scrolled up so the window is not at the tail — the answer's last rows have
/// left the pane — and the live answer is still on screen: its marker must
/// survive the scroll.
#[test]
fn the_draft_marker_survives_scrolling() {
    let mut app = overflowing_app();
    app.transcript.scroll_up(2, 78, 24);
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        !buffer.contains("arriving part c"),
        "precondition: the window moved up — the answer's tail left the pane:\n{buffer}"
    );
    assert!(
        buffer.contains("arriving part a"),
        "precondition: the live answer is still on screen:\n{buffer}"
    );
    assert!(
        buffer.contains("SAYA (draft)"),
        "the draft marker must survive scrolling:\n{buffer}"
    );
}

/// The control: a short conversation that fits its pane still marks its
/// draft. This passes before and after the fix — it proves nothing on its
/// own; it guards the no-regression flank of the change.
#[test]
fn a_short_conversation_still_marks_its_draft() {
    let mut app = empty_app();
    app.transcript.push(BlockKind::User, "count the orders");
    apply_event(
        &mut app.transcript,
        AgentEvent::assistant_text("the total is 42"),
        false,
    );
    app.request.stream = Some(busy_stream());
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        buffer.contains("SAYA (draft)"),
        "a short conversation's draft marker must not regress:\n{buffer}"
    );
}
