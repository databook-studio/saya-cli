//! The drop order itself: what gives way first, what gives way in words only,
//! and what never gives way at all.

use saya_agent::PermissionMode;

use super::status::{ActivityState, StatusFacts, status_line};
use super::status_fixture::{assert_row, cells, full, row};

/// The workspace is the first thing to go, and it goes alone: at 138 columns
/// the row is one cell over with it, and fits without it.
#[test]
fn the_workspace_goes_first_and_by_itself() {
    let line = status_line(&full(), 138);

    assert_row(
        &line,
        138,
        "ask │ demo +1 │ deepseek-v4.1-flash │ ⚠ host unsandboxed │ \
         ⚠ data sharing on │ ⊘ deny: curl,rm │ ctx 23% │ jobs 1",
        "● working 6s · Esc stop",
    );
    assert!(
        !row(&line).contains("~/shop"),
        "the workspace is the first segment to give way: {:?}",
        row(&line)
    );
}

/// A row too narrow for anything keeps the chip and both caveats and
/// overflows: a session that cannot say what it may touch is worse than a row
/// that runs past the edge.
#[test]
fn a_caveat_is_never_a_candidate_to_drop() {
    let line = status_line(&full(), 24);

    let text = row(&line);
    assert!(
        text.starts_with("ask"),
        "the mode chip never gives way: {text:?}"
    );
    assert!(
        text.contains("host unsandboxed"),
        "the unsandboxed caveat never gives way: {text:?}"
    );
    assert!(
        text.contains("⚠ sharing"),
        "the sharing caveat shortens but stays: {text:?}"
    );
    assert!(
        cells(&line) > 24,
        "the row overflows before it hides a caveat: {text:?}"
    );
}

/// One database has no shorter form, so its step leaves it saying its name
/// while everything droppable around it is gone.
#[test]
fn a_single_database_is_never_removed() {
    let facts = StatusFacts {
        databases: vec!["demo".to_owned()],
        model: "m".to_owned(),
        workspace: Some("~/shop".to_owned()),
        sharing: true,
        ..StatusFacts::default()
    };
    let line = status_line(&facts, 30);

    assert_row(&line, 30, "ask │ demo │ ⚠ sharing", "? help");
}

/// A toast that cannot fit is cut to the cells left over — the surface's own
/// words are the only ones the row will shorten, and even then the caveats
/// beside it stay whole.
#[test]
fn a_toast_is_cut_where_a_caveat_would_not_be() {
    let facts = StatusFacts {
        toast: Some("mode → plan: write tools refuse until you leave plan".to_owned()),
        ..full()
    };
    let line = status_line(&facts, 60);

    let text = row(&line);
    assert_eq!(cells(&line), 60, "a cut toast still fits the row: {text:?}");
    assert!(
        text.starts_with("ask │ +1 │ ⚠ host unsandboxed │ ⚠ sharing"),
        "the caveats are whole and untouched: {text:?}"
    );
    let toast = "mode → plan: write tools refuse until you leave plan";
    let tail = text.rsplit("⚠ sharing").next().unwrap_or_default();
    let cut = tail.trim_start();
    assert!(
        !cut.is_empty() && toast.starts_with(cut),
        "the toast is cut, never rewritten: {cut:?}"
    );
    assert!(
        cut.len() < toast.len(),
        "the toast gave way to the caveats rather than the other way round: {cut:?}"
    );
}

/// The right side is a state word with a hint, and the hint is what goes
/// first: the queued count stays when its key does not.
#[test]
fn the_hint_goes_before_the_state_word() {
    let facts = StatusFacts {
        activity: ActivityState::Queued { count: 1 },
        mode: PermissionMode::Plan,
        ..StatusFacts::default()
    };
    let wide = status_line(&facts, 60);
    assert!(
        row(&wide).ends_with("queued 1 · Ctrl+G drop"),
        "the hint rides beside the state word while there is room: {:?}",
        row(&wide)
    );

    let facts = StatusFacts {
        model: "deepseek-v4.1-flash".to_owned(),
        ..facts
    };
    let narrow = status_line(&facts, 30);

    let text = row(&narrow);
    assert!(
        text.ends_with("queued 1"),
        "the state word outlives its hint: {text:?}"
    );
    assert!(
        !text.contains("Ctrl+G drop"),
        "the hint is dropped first: {text:?}"
    );
}
