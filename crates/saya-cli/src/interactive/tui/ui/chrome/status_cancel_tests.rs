//! The cancel affordance stays on the busy bar at every supported width
//! (audit finding F06). The predecessor test in `ui_snapshot_gate_action.rs`
//! asserted span arithmetic instead of the painted frame and stayed green
//! while `(Esc to cancel)` went missing; these render a real frame through
//! `ui::draw` into a `ratatui` test backend and assert on the buffer's text —
//! nothing else.

use crate::interactive::tui::transcript::BlockKind;
use crate::interactive::tui::types::App;
use crate::interactive::tui::ui_snapshot_tests::{empty_app, fixed_status, render_buffer};

/// A running request whose stream never delivers — the same fixture the
/// snapshot harness uses, built locally because `support` is private to the
/// snapshot hub's descendants.
fn busy_stream() -> crate::interactive::tui::agent::Stream {
    let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
    crate::interactive::tui::agent::Stream {
        rx,
        cancel: saya_agent::CancellationToken::new(),
        prompt: String::new(),
    }
}

/// A busy app running `bounded_sql_query` against `sql`: the audit's
/// reproduction — the existing status fixture, a streaming request, one open
/// call whose target is the SQL.
fn busy_app(sql: &str) -> App {
    let mut app = empty_app();
    app.request.stream = Some(busy_stream());
    app.request.started = Some(std::time::Instant::now());
    app.request.activity = Some("bounded_sql_query".into());
    app.transcript.buffer_tool_request(
        "bounded_sql_query".into(),
        serde_json::json!({ "sql": sql }),
        None,
    );
    app
}

/// The audit's long SQL action: the same detail the weak predecessor test
/// pins, long past any frame the bar can carry.
fn long_sql() -> String {
    "select a_very_long_column_list from some_table ".repeat(20)
}

/// A reader scrolled up with rows landed below the fold, then the busy
/// request: the new-activity notice shares the row with the bar.
fn busy_app_with_unseen_rows(sql: &str) -> App {
    let mut app = empty_app();
    for i in 0..8 {
        app.transcript
            .push(BlockKind::Assistant, format!("line {i}"));
    }
    app.transcript.scroll_up(2, 100, 5);
    let _ = app.transcript.view(100, 5);
    app.transcript
        .push(BlockKind::Assistant, "a late answer lands below the fold");
    assert!(
        app.unseen_new_rows() > 0,
        "precondition: the fixture must hold unseen rows or no notice can paint"
    );
    app.request.stream = Some(busy_stream());
    app.request.started = Some(std::time::Instant::now());
    app.request.activity = Some("bounded_sql_query".into());
    app.transcript.buffer_tool_request(
        "bounded_sql_query".into(),
        serde_json::json!({ "sql": sql }),
        None,
    );
    app
}

/// The audit's exact reproduction: 100 columns, the fixed status fixture, a
/// long SQL action. Whatever the detail's length, `(Esc to cancel)` must be
/// in the final rendered buffer — the one affordance naming how to stop.
#[test]
fn cancel_hint_survives_a_long_action_at_100_columns() {
    let app = busy_app(&long_sql());
    let buffer = render_buffer(&app, &fixed_status(), 100, 10);
    assert!(
        buffer.contains("…"),
        "precondition: the long detail must truncate with an ellipsis:\n{buffer}"
    );
    assert!(
        buffer.contains("Esc to cancel"),
        "the cancel affordance must paint beside a long action at 100 columns:\n{buffer}"
    );
}

/// The narrower supported width. The bar sheds before the stop affordance
/// does, whatever the detail's length.
#[test]
fn cancel_hint_survives_at_80_columns() {
    let app = busy_app(&long_sql());
    let buffer = render_buffer(&app, &fixed_status(), 80, 10);
    assert!(
        buffer.contains("Esc to cancel"),
        "the cancel affordance must survive an 80-column frame:\n{buffer}"
    );
}

/// The unseen-count span shares the row with the busy bar; the audit calls
/// this arrangement out specifically. The notice still leads, and the hint
/// still paints.
#[test]
fn cancel_hint_survives_beside_a_new_activity_notice() {
    let app = busy_app_with_unseen_rows(&long_sql());
    let buffer = render_buffer(&app, &fixed_status(), 100, 10);
    assert!(
        buffer.contains("new line"),
        "the new-activity notice must share the busy row:\n{buffer}"
    );
    assert!(
        buffer.contains("Esc to cancel"),
        "the cancel affordance must survive beside the notice:\n{buffer}"
    );
}

/// The control: a short action on a wide frame paints the action, the status
/// details and the hint together. Guards against a fix that always sheds the
/// details to keep the arithmetic simple — green before the fix, so it is a
/// guard, not evidence.
#[test]
fn a_short_action_at_a_wide_width_paints_details_and_hint_together() {
    let app = busy_app("select region, count(*) from orders");
    let buffer = render_buffer(&app, &fixed_status(), 200, 12);
    assert!(
        buffer.contains("running bounded_sql_query: select region, count(*) from orders"),
        "the short action paints in full at a wide frame:\n{buffer}"
    );
    assert!(
        buffer.contains("approval:read-only"),
        "the approval segment still paints when there is room:\n{buffer}"
    );
    assert!(
        buffer.contains("ws:/home/user/proj"),
        "the workspace segment still paints when there is room:\n{buffer}"
    );
    assert!(
        buffer.contains("sharing:on"),
        "the sharing segment still paints when there is room:\n{buffer}"
    );
    assert!(
        buffer.contains("Esc to cancel"),
        "the cancel affordance paints beside the full tail:\n{buffer}"
    );
}
