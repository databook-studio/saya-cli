//! The transcript's step lines (`02` §3.2): one compact line per step carrying
//! the command, path or query it ran, the one key that opens every step in full
//! with the outcome it reported, the cap on a long turn, the spinner of a step
//! in flight, and the wrap at a narrow width.
//!
//! Every assertion here is about a row an operator reads: what a turn looks like
//! when it settles is the whole point of this pane, so the rows are read as a
//! terminal paints them.

use serde_json::json;

use super::transcript_steps_fixture::{
    COMMAND, PATH, done, facts, live_turn, many_step_turn, rows_in, running, step_lines, step_rows,
    two_step_turn,
};
use crate::front::tui::focus::EntryPath;

#[test]
fn a_settled_turn_shows_a_line_for_every_step_with_what_it_ran() {
    let rows = rows_in(&two_step_turn(), &facts(false), 100, 30);

    assert_eq!(
        step_lines(&rows).len(),
        2,
        "two steps paint two lines, and the turn settles without folding them \
         away: {rows:?}"
    );
    assert!(
        rows.iter().any(|row| row.contains("python3 -m unittest")),
        "the command a step ran is on its line: {rows:?}"
    );
    assert!(
        rows.iter().any(|row| row.contains("Read pricing.py")),
        "a read names the file it read: {rows:?}"
    );
    assert!(
        rows.iter().any(|row| row.contains("2 steps")),
        "a settled turn keeps its summary header over the step lines: {rows:?}"
    );
    assert!(
        !rows.iter().any(|row| row.contains("python3 exited 1")),
        "the outcome stays folded in the compact layout: {rows:?}"
    );
}

#[test]
fn the_detail_key_reveals_every_steps_outcome() {
    let compact = rows_in(&two_step_turn(), &facts(false), 100, 30);
    let detail = rows_in(&two_step_turn(), &facts(true), 100, 30);

    assert!(
        !compact.iter().any(|row| row.contains("python3 exited 1")),
        "compact: the outcome is folded: {compact:?}"
    );
    for outcome in ["python3 exited 1", "read 40 lines"] {
        assert!(
            detail.iter().any(|row| row.contains(outcome)),
            "detail: the outcome a step reported is under its line ({outcome:?}): {detail:?}"
        );
    }
    assert!(
        detail.len() > compact.len(),
        "detail shows more rows than compact: {} vs {}",
        detail.len(),
        compact.len()
    );
}

#[test]
fn the_group_header_names_the_detail_key() {
    let rows = rows_in(&two_step_turn(), &facts(false), 100, 30);
    let header = rows
        .iter()
        .find(|row| row.contains("2 steps"))
        .unwrap_or_else(|| panic!("the group paints its summary header: {rows:?}"));

    assert!(
        header.contains("Ctrl+O detail"),
        "the header names the one key that opens the steps: {header:?}"
    );
    let detail = rows_in(&two_step_turn(), &facts(true), 100, 30);
    assert!(
        detail.iter().any(|row| row.contains("Ctrl+O compact")),
        "and names the way back while full detail is on: {detail:?}"
    );
}

#[test]
fn a_focused_group_names_the_enter_key_instead() {
    let mut facts = facts(false);
    facts.focus = Some(EntryPath::new(0, 1));
    let rows = rows_in(&two_step_turn(), &facts, 100, 30);
    let header = rows
        .iter()
        .find(|row| row.contains("2 steps"))
        .unwrap_or_else(|| panic!("the group paints its summary header: {rows:?}"));

    assert!(
        header.contains("Enter fold"),
        "a selected entry names the key that acts on it: {header:?}"
    );
    assert!(
        !header.contains("Ctrl+O"),
        "and the selected entry's own key takes the hint: {header:?}"
    );
}

#[test]
fn a_turn_of_many_steps_shows_the_last_five_and_counts_the_rest() {
    let rows = rows_in(&many_step_turn(8), &facts(false), 100, 30);

    assert_eq!(
        step_lines(&rows).len(),
        5,
        "a long turn keeps five step lines on screen: {rows:?}"
    );
    assert!(
        rows.iter().any(|row| row.contains("+3 earlier steps")),
        "and counts the ones it keeps off screen: {rows:?}"
    );
    assert!(
        rows.iter().any(|row| row.contains("file8.py")),
        "the newest step is the one kept: {rows:?}"
    );
    assert!(
        !rows.iter().any(|row| row.contains("file1.py")),
        "the oldest steps are the ones counted: {rows:?}"
    );
}

#[test]
fn a_narrow_transcript_wraps_the_whole_command_without_cutting_a_word() {
    let rows = rows_in(&two_step_turn(), &facts(true), 60, 24);

    for word in COMMAND.split_whitespace() {
        assert!(
            rows.iter().any(|row| row.contains(word)),
            "every word of the command survives 60 columns ({word:?}): {rows:?}"
        );
    }
    for row in &rows {
        assert!(
            cell_width(row) <= 60,
            "a row wider than 60 columns: {row:?}"
        );
    }
    // The compact line is cut to the width, and cut at a word: what survives
    // ends where a word ends, so no word is left half-written.
    let compact = rows_in(&two_step_turn(), &facts(false), 60, 24);
    let line = compact
        .iter()
        .find(|row| row.contains("python3 -m unittest"))
        .unwrap_or_else(|| panic!("the command's compact line paints: {compact:?}"));
    let cut = line
        .trim_start_matches(' ')
        .split_once("✓ ")
        .map(|(_, words)| words)
        .unwrap_or(line)
        .trim();
    let kept = cut
        .strip_prefix("Run ")
        .unwrap_or(cut)
        .trim_end_matches('…')
        .trim_end();
    let at = COMMAND
        .find(kept)
        .unwrap_or_else(|| panic!("the line is the command's own words: {cut:?}"));
    assert!(
        COMMAND[at + kept.len()..].starts_with(' '),
        "the cut lands between two words: {cut:?}"
    );
}

#[test]
fn a_running_step_turns_a_spinner() {
    let live = live_turn(vec![
        done("shell", json!({"command": COMMAND}), "python3 exited 1"),
        running("read", json!({"path": PATH})),
    ]);
    let first = rows_in(&live, &facts(false), 100, 30);
    let mut later_facts = facts(false);
    later_facts.spinner = 1;
    let later = rows_in(&live, &later_facts, 100, 30);

    assert_ne!(
        first, later,
        "a step in flight is drawn from the tick's frame, so it moves: {first:?}"
    );
    let step = step_rows(&first)
        .last()
        .cloned()
        .unwrap_or_else(|| panic!("the running step paints a line: {first:?}"));
    assert!(
        step.contains("Read pricing.py"),
        "the running step still names what it is doing: {step:?}"
    );

    let mut ascii = facts(false);
    ascii.spinner = 1;
    ascii.charset = crate::front::tui::theme::Charset::Ascii;
    let drawn = rows_in(&live, &ascii, 100, 30);
    let frame = step_rows(&drawn)
        .last()
        .cloned()
        .unwrap_or_else(|| panic!("the running step paints a line: {drawn:?}"));
    let mark = frame
        .trim_start_matches(' ')
        .chars()
        .next()
        .unwrap_or_default();
    assert!(
        mark.is_ascii() && "|/-\\".contains(mark),
        "the ascii spinner is a seven-bit frame: {frame:?}"
    );
}

/// The cells `row` occupies, as a terminal counts them.
fn cell_width(row: &str) -> usize {
    crate::interactive::tui::wrap::cell_width(row)
}
