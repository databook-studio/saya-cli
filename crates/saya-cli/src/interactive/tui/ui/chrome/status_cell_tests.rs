//! The busy bar's cell accounting (re-audit R01): every width the row is
//! planned against is terminal cells, never `char`s. Slice D pinned the
//! cancel affordance with ASCII, where a char is a cell; a CJK char paints
//! two cells, so wide text reopened the defect. These render real frames and
//! assert the painted buffer — the seam `status_cancel_tests` established —
//! plus the detail truncation's units through `action_text` directly.

use super::super::action_line::action_text;
use crate::interactive::tui::types::App;
use crate::interactive::tui::ui_snapshot_tests::{empty_app, fixed_status, render_buffer};
use unicode_width::UnicodeWidthStr;

/// A running request whose stream never delivers — the same fixture
/// `status_cancel_tests` builds locally, for the same privacy reason.
fn busy_stream() -> crate::interactive::tui::agent::Stream {
    let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
    crate::interactive::tui::agent::Stream {
        rx,
        cancel: saya_agent::CancellationToken::new(),
        prompt: String::new(),
    }
}

/// A busy app running `bounded_sql_query` against `sql`: the audit's
/// reproduction — a streaming request, one open call whose target is the SQL.
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

/// Slice D's case with a wide target: 30 CJK pairs are 60 `char`s but 120
/// cells — double what a char count claims, so the reserved cancel hint is
/// pushed off the row again. The painted buffer is the judge.
#[test]
fn the_cancel_hint_survives_a_long_cjk_action_at_100_columns() {
    let app = busy_app(&"日本".repeat(30));
    let buffer = render_buffer(&app, &fixed_status(), 100, 10);
    assert!(
        buffer.contains("…"),
        "precondition: the wide detail must truncate with an ellipsis:\n{buffer}"
    );
    assert!(
        buffer.contains("Esc to cancel"),
        "the cancel affordance must paint beside a long CJK action at 100 columns:\n{buffer}"
    );
}

/// The narrower supported width, wide text: the row sheds before the stop
/// affordance does, whatever units the detail is counted in.
#[test]
fn the_cancel_hint_survives_a_long_cjk_action_at_80_columns() {
    let app = busy_app(&"日本".repeat(30));
    let buffer = render_buffer(&app, &fixed_status(), 80, 10);
    assert!(
        buffer.contains("Esc to cancel"),
        "the cancel affordance must survive an 80-column frame with wide text:\n{buffer}"
    );
}

/// The control — green on the unfixed tree, so a guard isolating the unit
/// rather than evidence: an ASCII target of the same *cell* length (120) as
/// the CJK case paints the hint, because for ASCII a char count is a cell
/// count. Twin of the 100-column CJK case above.
#[test]
fn an_equal_length_ascii_action_still_paints_the_hint() {
    let app = busy_app(&"a".repeat(120));
    let buffer = render_buffer(&app, &fixed_status(), 100, 10);
    assert!(
        buffer.contains("…"),
        "precondition: the equal-length ASCII detail must truncate too:\n{buffer}"
    );
    assert!(
        buffer.contains("Esc to cancel"),
        "the control: an ASCII action of equal cell length must keep the hint:\n{buffer}"
    );
}

/// The detail head's units: the target truncates to a cell budget, so a cut
/// lands between graphemes, never inside one — a base never loses its
/// combining mark and a ZWJ sequence is never split, even when the budget
/// ends mid-cluster.
#[test]
fn a_truncated_detail_never_splits_a_grapheme() {
    // `e` + U+0301 is one grapheme: two `char`s, one cell. A char-counted
    // cut at 9 lands between the base and its mark.
    let mark = "e\u{0301}";
    let marked = mark.repeat(20);
    let result = action_text(
        Some("bounded_sql_query"),
        Some((
            "bounded_sql_query".into(),
            serde_json::json!({ "sql": marked }),
        )),
        38,
    );
    assert_eq!(
        result,
        format!("running bounded_sql_query: {}… ", mark.repeat(9)),
        "the cut must keep whole base+mark clusters:\n{result}"
    );
    // The ZWJ family is one grapheme riding eight ASCII cells, with the
    // budget ending mid-sequence: the family is dropped whole or not at all.
    let family = "👨\u{200D}👩\u{200D}👦";
    let detail = format!("{}{family}b", "a".repeat(8));
    let result = action_text(
        Some("bounded_sql_query"),
        Some((
            "bounded_sql_query".into(),
            serde_json::json!({ "sql": detail }),
        )),
        38,
    );
    assert_eq!(
        result,
        format!("running bounded_sql_query: {}… ", "a".repeat(8)),
        "a grapheme too wide to finish in the budget is dropped whole:\n{result}"
    );
    assert!(
        !result.contains('\u{1F468}') && !result.contains('\u{200D}'),
        "no fragment of the family may survive the cut:\n{result}"
    );
}

/// The truncated detail is measured in cells, ellipsis glyph included: the
/// painted phrase fits the room the plan handed it, not a char count of it.
#[test]
fn a_truncated_detail_never_exceeds_its_cell_budget() {
    let wide = "日".repeat(50);
    let result = action_text(
        Some("bounded_sql_query"),
        Some((
            "bounded_sql_query".into(),
            serde_json::json!({ "sql": wide }),
        )),
        40,
    );
    assert_eq!(
        result,
        format!("running bounded_sql_query: {}… ", "日".repeat(5)),
        "the cut honours the cell budget exactly:\n{result}"
    );
    assert!(
        result.width() <= 40,
        "the painted phrase must fit its 40-cell room ({} cells): {result:?}",
        result.width()
    );
}
