//! Re-audit slice R03: a shrinking mutation below a scrolled reader must not
//! move the reader. The fold of a completed tool run and the discard of a
//! buffered run both remove rows from the tail; the boundary used to compute
//! `growth == 0` for them, so `scroll_up` never fell and the tail-relative
//! window slid up through history under a reader who pressed nothing. Every
//! test here renders, mutates, renders — and asserts on the **visible rows**,
//! never on `scroll_up` alone: an offset that holds while the reader still
//! moves is exactly the failure this slice closes.

use crate::interactive::tui::transcript::MAX_BLOCKS;
use crate::interactive::tui::transcript::{Block, BlockKind, Transcript, chapters};

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

fn flush(t: &mut Transcript) {
    t.flush_tool_buffer(
        |name, args| vec![format!("→ {name} {args}")],
        |name, summary| format!("✓ {name}: {summary}"),
    );
}

/// The bare history: twenty assistant blocks (label + body = two painted rows
/// each), height 5, scrolled back 15, so `start = 40 - 5 - 15 = 20` — the
/// re-audit's arithmetic. The trailing `view` warms the wrap cache the
/// boundary freezes from.
fn scrolled_history() -> Transcript {
    let mut t = Transcript::new();
    for i in 0..20 {
        t.push(BlockKind::Assistant, format!("history line {i}"));
    }
    t.scroll_up(15, W, H);
    assert!(
        !t.is_following_tail(),
        "precondition: the reader scrolled up"
    );
    assert_eq!(
        view_texts(&t, H),
        ["SAYA", "history line 10", "SAYA", "history line 11", "SAYA",],
        "precondition: the window sits on painted rows 20…24"
    );
    t
}

/// [`scrolled_history`] plus a completed two-call tool run buffered at the
/// tail: painted rows 40…47, strictly below the five-row window at 20…24
/// (the append path holds the anchor at row 20 while they land).
fn scrolled_history_with_run() -> Transcript {
    let mut t = scrolled_history();
    t.buffer_tool_request("workspace_write".into(), write_request("north.md"), None);
    t.buffer_tool_request("workspace_write".into(), write_request("south.md"), None);
    assert!(t.buffer_tool_completion("workspace_write", "north.md written"));
    assert!(t.buffer_tool_completion("workspace_write", "south.md written"));
    assert!(
        t.blocks()
            .last()
            .is_some_and(|b| b.text.contains("✓ workspace_write")),
        "precondition: the completed run sits at the tail, below the window"
    );
    t
}

/// Core test (re-audit R03): the fold collapses the run below the window into
/// one summary block — rows vanish from the tail — and the reader must not
/// move. Before the fix the boundary computed `growth == 0` for the shrink,
/// `scroll_up` stayed, and `start` fell with `rem`, sliding the window up
/// through history.
#[test]
fn a_group_collapsing_below_the_viewport_does_not_move_the_reader() {
    let mut t = scrolled_history_with_run();
    let before = view_texts(&t, H);
    flush(&mut t);
    let last = t.blocks().last().expect("blocks remain");
    assert!(
        last.is_collapsible() && last.text.contains("2 tool calls"),
        "precondition: the run folded into one summary below the window: {}",
        last.text
    );
    assert_eq!(
        view_texts(&t, H),
        before,
        "the collapse below the window must not move the visible rows"
    );
}

/// Same probe through `discard_tool_buffer` (the `TurnReset` retry path): the
/// run's live rows drop off the tail below the window; the reader stays.
#[test]
fn a_discarded_tool_buffer_below_the_viewport_does_not_move_the_reader() {
    let mut t = scrolled_history_with_run();
    let before = view_texts(&t, H);
    t.discard_tool_buffer();
    assert!(
        t.blocks()
            .last()
            .is_some_and(|b| b.text.contains("history line 19")),
        "precondition: the run's live rows were discarded from the tail"
    );
    assert_eq!(
        view_texts(&t, H),
        before,
        "the discard below the window must not move the visible rows"
    );
}

/// The other shrink: `enforce_bounds` drains rows off the FRONT while
/// scrolled. `scroll_up` must not move for the eviction — the whole array
/// shifts down by the dropped rows, so `start` falling by that same count
/// already lands on the same logical rows. One signed delta measured over the
/// whole mutation would double-count here; only the closure's own delta may
/// touch `scroll_up`.
#[test]
fn prefix_eviction_does_not_double_adjust() {
    let mut t = Transcript::new();
    for i in 0..MAX_BLOCKS {
        t.push(BlockKind::User, format!("m{i}"));
    }
    let _ = view_texts(&t, H);
    t.scroll_up(15, W, H);
    assert!(
        !t.is_following_tail(),
        "precondition: the reader scrolled up"
    );
    let before = view_texts(&t, H);
    t.push(BlockKind::User, "one more");
    assert_eq!(
        t.blocks().first().map(|b| b.text.as_str()),
        Some("m1"),
        "precondition: the bound evicted rows off the front"
    );
    assert_eq!(
        view_texts(&t, H),
        before,
        "front eviction must land the window on the same logical rows"
    );
}

/// A mutation that both removes and adds rows below the window nets into ONE
/// delta applied to `scroll_up`: three blocks drop off the tail (six rows),
/// one lands (two rows), net −4. The anchor holds and `scroll_up` ends at
/// exactly baseline − 4 — not zero, not −6.
#[test]
fn a_shrink_and_an_append_in_one_mutation_net_correctly() {
    let mut t = scrolled_history();
    let before = view_texts(&t, H);
    let anchor = t.scroll_up;
    t.mutate(|t| {
        t.blocks.pop();
        t.blocks.pop();
        t.blocks.pop();
        let chapter = chapters::chapter_for(&t.blocks, BlockKind::Tool);
        t.blocks.push(Block {
            kind: BlockKind::Tool,
            text: "tail work landed".to_string(),
            chapter,
            group: None,
        });
    });
    assert_eq!(
        t.blocks().last().map(|b| b.text.as_str()),
        Some("tail work landed"),
        "precondition: the mutation both removed and added below the window"
    );
    assert_eq!(
        view_texts(&t, H),
        before,
        "the net shrink must not move the visible rows"
    );
    assert_eq!(
        t.scroll_up,
        anchor - 4,
        "one net delta: six rows out, two rows in"
    );
}

/// Control: at the tail a collapse behaves exactly as today — the folded
/// summary surfaces on the very next render, nothing is compensated, nothing
/// is counted.
#[test]
fn tail_follow_is_unchanged_by_a_collapse() {
    let mut t = Transcript::new();
    for i in 0..20 {
        t.push(BlockKind::Assistant, format!("history line {i}"));
    }
    t.buffer_tool_request("workspace_write".into(), write_request("north.md"), None);
    t.buffer_tool_request("workspace_write".into(), write_request("south.md"), None);
    assert!(t.buffer_tool_completion("workspace_write", "north.md written"));
    assert!(t.buffer_tool_completion("workspace_write", "south.md written"));
    let _ = view_texts(&t, H);
    assert!(
        t.is_following_tail(),
        "precondition: watching the live tail"
    );
    flush(&mut t);
    assert!(t.is_following_tail(), "the tail anchor must stay at zero");
    assert_eq!(t.unseen_new_rows(), 0, "nothing is counted at the tail");
    let shown = view_texts(&t, H);
    assert!(
        shown.iter().any(|l| l.contains("2 tool calls")),
        "the folded summary must surface on the very next render: {shown:?}"
    );
}

/// `unseen_rows` counts new activity: it rises on growth and must not rise on
/// a shrink — a collapsing group is not new activity. Decided semantics: it
/// falls by the shrink, clamped to `scroll_up`. The run's live rows are the
/// newest rows below the fold — the exact rows the count was raised for — so
/// when they fold away the promise must shrink with them, or the count stops
/// tracking anything painted; the clamp to `scroll_up` (which, by the
/// tail-relative window identity, is exactly the count of rows actually below
/// the window) keeps the bound in every other case.
#[test]
fn unseen_rows_does_not_rise_on_a_collapse() {
    let mut t = scrolled_history_with_run();
    let counted = t.unseen_new_rows();
    assert_eq!(
        counted, 8,
        "precondition: the run streamed below the scrolled reader, four \
         blocks of two rows each"
    );
    flush(&mut t);
    let after = t.unseen_new_rows();
    assert!(
        after <= counted,
        "a collapsing group is not new activity: {after} > {counted}"
    );
    assert_eq!(
        after, 2,
        "the fold removed six of the eight counted rows (the run's live rows \
         folded into one two-row block), so the count falls with them"
    );
    let (total, start) = t.scroll_metrics(W, H);
    assert!(
        after <= total - start - H,
        "the count must never exceed the rows actually below the window: \
         {after} > {}",
        total - start - H
    );
}
