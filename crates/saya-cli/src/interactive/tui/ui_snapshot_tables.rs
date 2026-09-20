use super::super::stream_events::apply_event;
use super::super::transcript::BlockKind;
/// Composed-screen behaviour snapshots (wide result tables and long content).
/// Moved byte-identical from the hub; plain assertions except the renamed long-content snapshot.
use super::claims::{app_with_wide_table, supplied_claim, supplied_contract};
use super::support::{bounded_sql_query_effect, empty_app, fixed_status, render_buffer};
use saya_agent::{AgentEvent, KnowledgeOutcome};
use saya_types::ClaimStatus;

/// At the left edge the first columns are painted and the last column is off
/// the right side; scrolling right reveals it. This asserts real paint through
/// `ui::draw` (the same path the PTY smoke tests exercise at the process level).
#[test]
fn wide_table_scrolls_horizontally_in_the_transcript() {
    let mut app = app_with_wide_table();
    app.wide_table.h_offset = 0;

    let left = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        left.contains("id"),
        "first column is visible at the left edge:\n{left}"
    );
    assert!(
        !left.contains("shipped_at"),
        "last column does not fit before scrolling:\n{left}"
    );

    // Scroll far enough that the early columns leave the window.
    app.wide_table.h_offset = 11;
    let right = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        right.contains("shipped_at"),
        "scrolling right reveals the last column:\n{right}"
    );
    assert!(
        !right.contains("│ id"),
        "scrolling right drops the first column:\n{right}"
    );
}

/// Pinning the first column holds it in place while the rest scroll, so the
/// key column (an id) never leaves the screen while reading wide rows.
#[test]
fn pin_first_column_stays_put_while_scrolling() {
    let mut app = app_with_wide_table();
    app.wide_table.pin_first = true;
    app.wide_table.h_offset = 11;

    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        buffer.contains("│ id"),
        "pinned first column stays on screen:\n{buffer}"
    );
    assert!(
        buffer.contains("shipped_at"),
        "a far column is reached by scrolling:\n{buffer}"
    );
}

/// The view offset is presentation only: the transcript block text is the full
/// untruncated table, so copy (Ctrl+B) still yields every column even while the
/// screen is scrolled and clipped.
#[test]
fn copy_transcript_yields_the_full_untruncated_table_while_scrolled() {
    let mut app = app_with_wide_table();
    app.wide_table.h_offset = 11;
    app.wide_table.pin_first = true;

    app.copy_transcript();
    let copied = app.pending_clipboard.expect("transcript was queued");
    assert!(
        copied.contains("id") && copied.contains("name") && copied.contains("shipped_at"),
        "copy must include every column, not just the visible window:\n{copied}"
    );
    assert!(
        copied.contains("1 row(s)"),
        "copy must include the row-count footer:\n{copied}"
    );
}

/// `copy_last_answer` copies the assistant answer, not the table; the table is
/// never mistaken for the answer. (Guards the new BlockKind against regressing
/// the copy-last-answer path.)
#[test]
fn copy_last_answer_does_not_grab_a_table_block() {
    let mut app = app_with_wide_table();
    app.transcript
        .push(BlockKind::Assistant, "the answer is here");
    app.copy_last_answer();
    let copied = app.pending_clipboard.expect("answer was queued");
    assert_eq!(copied, "the answer is here");
}

// --- Screen 3: long content at a real width (wrap + truncation). -------------

/// A wide SQL block (a `WHERE` clause wider than the text area) and a long
/// claim value, at 100×30. The SQL wraps across rows; the claim value wraps
/// too; and because the whole exceeds the transcript height, the top is
/// truncated (only the tail is visible). This is where layout regressions hide.
#[test]
fn long_content_at_real_width() {
    let mut app = empty_app();
    // A wide SQL block: format_sql puts each clause on its own line; the long
    // SELECT list and WHERE each wrap at the text width (98).
    apply_event(
        &mut app.transcript,
        AgentEvent::tool_requested(
            "bounded_sql_query",
            serde_json::json!({
                "sql": "select order_id, customer_id, placed_at, fulfilled_at, shipped_at, total_amount, tax_amount, discount_amount, currency, status, region, country, city, postal_code, carrier, tracking_number from catalog.public.orders where placed_at >= '2024-01-01' and status in ('fulfilled','shipped','delivered') and total_amount > 100 and region in ('north','south','east','west','central','pacific','mountain') and currency = 'USD' and carrier is not null order by placed_at desc, total_amount desc limit 50",
                "connection": "analytics",
            }),
            Some(bounded_sql_query_effect()),
        ),
        false,
    );
    apply_event(
        &mut app.transcript,
        AgentEvent::ToolCompleted {
            name: "bounded_sql_query".into(),
            summary: "50 rows".into(),
        },
        false,
    );
    // A second wide SQL block so the transcript overflows the 26-row region at
    // 100×30: the tail-view truncates the top, cutting off the first SQL header.
    apply_event(
        &mut app.transcript,
        AgentEvent::tool_requested(
            "bounded_sql_query",
            serde_json::json!({
                "sql": "select customer_id, count(*) as orders, sum(total_amount) as spend, avg(total_amount) as avg_order, max(placed_at) as last_order from catalog.public.orders where placed_at >= '2024-01-01' and status in ('fulfilled','shipped','delivered') group by customer_id having count(*) > 1 order by spend desc limit 25",
                "connection": "analytics",
            }),
            Some(bounded_sql_query_effect()),
        ),
        false,
    );
    apply_event(
        &mut app.transcript,
        AgentEvent::ToolCompleted {
            name: "bounded_sql_query".into(),
            summary: "25 rows".into(),
        },
        false,
    );
    apply_event(
        &mut app.transcript,
        AgentEvent::assistant_text(
            "Here are the fulfilled orders and the top spenders over 100 USD.",
        ),
        false,
    );
    // A long claim value: the supplied path renders the value raw (no eliding),
    // so a 180-char description wraps across several lines.
    apply_event(
        &mut app.transcript,
        AgentEvent::knowledge_supplied(
            KnowledgeOutcome::Ran {
                store_unavailable: false,
            },
            vec![supplied_contract(
                "analytics",
                "catalog.public.customer_notes",
                "current",
                vec![supplied_claim(
                    "ki-longdesc1",
                    "table_description",
                    "Free-text notes captured by support agents during customer interactions including follow-up reminders, escalation flags, and the internal handling summary used by the tier-two team when triaging escalations, plus the resolved-action log",
                    None,
                    ClaimStatus::Confirmed,
                )],
            )],
            0,
        ),
        false,
    );

    let buffer = render_buffer(&app, &fixed_status(), 100, 30);
    insta::assert_snapshot!(buffer);
}
