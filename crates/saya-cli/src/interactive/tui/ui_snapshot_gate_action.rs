use super::super::transcript::BlockKind;
/// Composed-screen behaviour snapshots (the no-colour gate and the action line).
/// Moved byte-identical from the hub; plain assertions only.
use super::support::{busy_stream, empty_app, fixed_status, render_buffer};

/// The phase gate: nothing essential relies on hue alone.
///
/// This asserts it directly rather than by toggling the palette. `render_buffer`
/// returns the `TestBackend` symbol view, which has already discarded every
/// `Style` — so if each kind is identifiable in *this* string, colour cannot be
/// carrying the distinction. Rendering twice with colour on and off and
/// comparing these buffers would instead compare two values that never held
/// colour in the first place: equal by construction, and green whatever the
/// renderer did.
///
/// `Error`, `System` and `Thinking` are identified by a glyph rather than a
/// word. That is the deliberate interim state — their label words are undecided
/// and belong to later phases — not an oversight for this test to paper over.
#[test]
fn every_block_kind_is_distinguishable_without_colour() {
    use super::super::transcript::BlockKind;
    let mut app = empty_app();
    for (kind, text) in [
        (BlockKind::User, "the user request"),
        (BlockKind::Assistant, "the assistant answer"),
        (BlockKind::Tool, "the tool trail"),
        (BlockKind::Table, "the result grid"),
        (BlockKind::Error, "the failure"),
        (BlockKind::System, "the receipt"),
        (BlockKind::Thinking, "the reasoning"),
    ] {
        app.transcript.push(kind, text);
    }
    let buffer = render_buffer(&app, &fixed_status(), 100, 40);

    for (marker, what) in [
        ("YOU", "a user turn"),
        ("SAYA", "an assistant turn"),
        ("ACTIVITY", "the tool trail"),
        ("RESULT", "a result"),
        ("✗ the failure", "a failure"),
        ("· the receipt", "system content"),
        ("≈ the reasoning", "reasoning"),
    ] {
        assert!(
            buffer.contains(marker),
            "{what} must be identifiable from symbols alone, found no {marker:?}:\n{buffer}"
        );
    }
}

// --- Fieldnotes phase 4: the action line names its target; drafts are marked.

/// Objective A: the busy line names the tool and its target — the path for
/// `workspace_write`, the SQL for a query — via the shared `tool_call_detail`
/// seam. Name the action and its target, never a motive ("to …" is forbidden).
#[test]
fn the_action_line_names_the_tool_and_its_target() {
    let mut app = empty_app();
    app.transcript
        .push(BlockKind::User, "count the orders by region");
    app.request.stream = Some(busy_stream());
    app.request.started = Some(std::time::Instant::now());
    app.request.activity = Some("bounded_sql_query".into());
    app.transcript.buffer_tool_request(
        "bounded_sql_query".into(),
        serde_json::json!({"sql": "select region, count(*) from orders"}),
        None,
    );
    assert_eq!(
        app.transcript.open_tool_count(),
        1,
        "the fixture must hold one open call or the bar cannot name its target"
    );
    assert_eq!(
        crate::interactive::tui::stream_events::tool_call_detail(
            "bounded_sql_query",
            &serde_json::json!({"sql": "select region, count(*) from orders"}),
        )
        .as_deref(),
        Some("select region, count(*) from orders"),
        "the shared seam must surface the SQL for this fixture"
    );
    let buffer = render_buffer(&app, &fixed_status(), 100, 10);
    assert!(
        buffer.contains("running bounded_sql_query: select region, count(*) from orders"),
        "the action line must name the tool and its target:\n{buffer}"
    );
}

/// Objective A: when `tool_call_detail` returns `None`, the line falls back
/// to today's `running {tool}` exactly — no invented target.
#[test]
fn an_action_with_no_detail_falls_back_to_the_bare_tool_name() {
    let mut app = empty_app();
    app.transcript.push(BlockKind::User, "what can you see");
    app.request.stream = Some(busy_stream());
    app.request.started = Some(std::time::Instant::now());
    app.request.activity = Some("schema_discovery".into());
    app.transcript
        .buffer_tool_request("schema_discovery".into(), serde_json::json!({}), None);
    let buffer = render_buffer(&app, &fixed_status(), 100, 10);
    assert!(
        buffer.contains("running schema_discovery"),
        "the bare tool name must render when there is no detail:\n{buffer}"
    );
    assert!(
        !buffer.contains("running schema_discovery:"),
        "no colon and no invented target when the detail is None:\n{buffer}"
    );
}

/// Objective A: a long detail is truncated so it can never push the elapsed
/// time or `(Esc to cancel)` off the bar.
#[test]
fn a_long_detail_never_pushes_the_cancel_hint_off_the_bar() {
    let mut app = empty_app();
    app.transcript.push(BlockKind::User, "count everything");
    app.request.stream = Some(busy_stream());
    app.request.started = Some(std::time::Instant::now());
    app.request.activity = Some("bounded_sql_query".into());
    app.transcript.buffer_tool_request(
        "bounded_sql_query".into(),
        serde_json::json!({"sql": "select a_very_long_column_list from some_table ".repeat(20)}),
        None,
    );
    let buffer = render_buffer(&app, &fixed_status(), 100, 10);
    // Assert the painted frame, not the budget the renderer worked from.
    // This test previously computed a row width through a test-only seam,
    // discarded it, and checked only that *something* was truncated — so it
    // passed against a bar with no cancel hint on it at all, which is the
    // bar that shipped (audit F06). The hint is now reserved before the
    // elastic spans are sized, so it is in the buffer and can be asserted
    // there.
    assert!(
        buffer.contains("Esc to cancel"),
        "a long detail must not push the cancel affordance off the bar:\n{buffer}"
    );
    assert!(
        buffer.contains("…"),
        "the over-long detail truncates with an ellipsis rather than overflowing:\n{buffer}"
    );
}
