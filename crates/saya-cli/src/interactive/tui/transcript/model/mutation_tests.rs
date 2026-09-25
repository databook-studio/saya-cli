//! Audit slice G (F03): one mutation boundary for every path that mutates
//! blocks. The gate is interleaving an actual render between events — a
//! test that fires events and only inspects state at the end cannot see the
//! stale-cache defect, so these render between events.

use crate::interactive::tui::transcript::MAX_BLOCKS;
use crate::interactive::tui::transcript::{BlockKind, Transcript};

const W: usize = 80;
const H: usize = 5;

fn view_texts(t: &Transcript, height: usize) -> Vec<String> {
    t.view(W, height)
        .iter()
        .map(|row| row.text.clone())
        .collect()
}

fn write_request(path: &str) -> serde_json::Value {
    serde_json::json!({"path": path})
}

/// Thirty assistant blocks (label + body = two painted rows each) scrolled
/// so the five-row window sits on painted rows 25…29 — the audit's probe
/// shape. The trailing `view` also warms the wrap cache, the baseline the
/// boundary freezes from.
fn scrolled_history() -> Transcript {
    let mut t = Transcript::new();
    for i in 0..30 {
        t.push(BlockKind::Assistant, format!("history line {i}"));
    }
    t.scroll_up(30, W, H);
    assert!(
        !t.is_following_tail(),
        "precondition: the reader scrolled up"
    );
    assert_eq!(
        view_texts(&t, H),
        [
            "history line 12",
            "SAYA",
            "history line 13",
            "SAYA",
            "history line 14",
        ],
        "precondition: the window shows painted rows 25…29"
    );
    t
}

/// Core test: request, render, complete, render. Without a render between
/// the two events the stale wrap cache is invisible.
#[test]
fn a_completion_is_visible_on_the_very_next_render() {
    let mut t = Transcript::new();
    t.push(BlockKind::User, "write the note");
    t.buffer_tool_request("workspace_write".into(), write_request("notes.md"), None);
    let first = view_texts(&t, 10);
    assert!(
        first
            .iter()
            .any(|l| l.contains("→ workspace_write: notes.md")),
        "precondition: the request is painted: {first:?}"
    );
    assert!(
        t.buffer_tool_completion("workspace_write", "notes.md written"),
        "precondition: the completion pairs"
    );
    let second = view_texts(&t, 10);
    assert!(
        second
            .iter()
            .any(|l| l.contains("✓ workspace_write: notes.md written")),
        "the completion must be painted on the very next render: {second:?}"
    );
}

/// Same shape, failing completion. This is the one with real consequences:
/// a repaint before the next boundary event must not read a failed tool as
/// still running.
#[test]
fn a_failure_mark_is_visible_on_the_very_next_render() {
    let mut t = Transcript::new();
    t.push(BlockKind::User, "write the note");
    t.buffer_tool_request("workspace_write".into(), write_request("notes.md"), None);
    let _ = view_texts(&t, 10);
    assert!(
        t.buffer_tool_completion("workspace_write", "write failed: disk full"),
        "precondition: the completion pairs"
    );
    let second = view_texts(&t, 10);
    assert!(
        second
            .iter()
            .any(|l| l.contains("✗ workspace_write: write failed")),
        "the failure mark must be painted on the very next render: {second:?}"
    );
}

#[test]
fn a_tool_request_while_scrolled_does_not_move_the_viewport() {
    let mut t = scrolled_history();
    let before = view_texts(&t, H);
    t.buffer_tool_request("workspace_write".into(), write_request("notes.md"), None);
    assert!(
        t.blocks()
            .last()
            .is_some_and(|b| b.text.contains("→ workspace_write")),
        "precondition: the request landed below the fold"
    );
    assert_eq!(
        view_texts(&t, H),
        before,
        "a tool request while scrolled must not move the visible window: \
         history 25…29 must not become 27…31"
    );
}

#[test]
fn a_tool_request_while_scrolled_counts_as_unseen_activity() {
    let mut t = scrolled_history();
    assert_eq!(
        t.unseen_new_rows(),
        0,
        "precondition: nothing has arrived below yet"
    );
    t.buffer_tool_request("workspace_write".into(), write_request("notes.md"), None);
    assert!(
        t.unseen_new_rows() > 0,
        "a request below a scrolled reader must raise the count through the \
         one raise site"
    );
}

/// Control: passes before and after the boundary lands. FIFO pairing of
/// same-name parallel calls is the deliberate rule documented in
/// `tool_buffer.rs`; pinned here so the refactor cannot quietly re-order it.
#[test]
fn same_name_parallel_calls_still_pair_fifo() {
    let mut t = Transcript::new();
    t.push(BlockKind::User, "write two notes");
    t.buffer_tool_request("workspace_write".into(), write_request("north.md"), None);
    t.buffer_tool_request("workspace_write".into(), write_request("south.md"), None);
    assert!(t.buffer_tool_completion("workspace_write", "north.md written"));
    assert_eq!(
        t.open_tool_count(),
        1,
        "the first completion closed the oldest open call, not the newest"
    );
    assert!(t.buffer_tool_completion("workspace_write", "south.md written"));
    assert_eq!(t.open_tool_count(), 0);
    t.flush_tool_buffer(
        |name, args| vec![format!("→ {name} {args}")],
        |name, summary| format!("✓ {name}: {summary}"),
    );
    let text = t
        .blocks()
        .iter()
        .flat_map(|b| {
            let mut v = vec![b.text.clone()];
            if let Some(g) = &b.group {
                v.extend(g.detail.clone());
            }
            v
        })
        .collect::<Vec<_>>()
        .join("\n");
    let north_q = text
        .find(r#"→ workspace_write {"path":"north.md""#)
        .expect("north request");
    let north_r = text
        .find("✓ workspace_write: north.md written")
        .expect("north result");
    let south_q = text
        .find(r#"→ workspace_write {"path":"south.md""#)
        .expect("south request");
    let south_r = text
        .find("✓ workspace_write: south.md written")
        .expect("south result");
    assert!(
        north_q < north_r && north_r < south_q && south_q < south_r,
        "each result must follow its own request:\n{text}"
    );
}

#[test]
fn bounds_hold_after_a_completion() {
    let mut t = Transcript::new();
    for i in 0..MAX_BLOCKS {
        t.push(BlockKind::User, format!("m{i}"));
    }
    t.buffer_tool_request("workspace_write".into(), write_request("notes.md"), None);
    assert!(
        t.blocks().len() <= MAX_BLOCKS,
        "precondition: the request path respects the bound"
    );
    t.buffer_tool_completion("workspace_write", "notes.md written");
    assert!(
        t.blocks().len() <= MAX_BLOCKS,
        "MAX_BLOCKS must hold after a completion too"
    );
}
