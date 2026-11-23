//! The panel's place line at the two review widths: a long folder keeps its own
//! name, a folder under the home directory reads as `~`, and the lane fact a
//! shell's place states stays on the panel — beside the folder when it fits, on
//! a row of its own when it does not.
//!
//! The folder is shortened from the front and never from the end: the last
//! component is the part an operator recognises, so it is the part the line
//! keeps whole.

use super::cards::{approval, cells};
use super::cards_fixture::{SCREEN_ROWS, scope_question, shell_question};
use crate::chat::Question;

/// A launch folder the shape a macOS temporary checkout has: 92 cells, so the
/// panel's 94-cell line at 100 columns cannot hold `in ` and all of it.
const LONG_FOLDER: &str =
    "/private/var/folders/p8/sz9h6r3x0h3gw9sr_dz6sglm0000gn/T/saya-approval-panel-wide-87382/repo";

/// The lane fact a shell's place states, and never drops.
const NOT_SANDBOXED: &str = "on your machine, not sandboxed";

/// The room one row inside the panel has at `cols` columns: the frame's two
/// gutters, the box's two rules and the cell of air beside each.
fn room(cols: usize) -> usize {
    cols - 6
}

/// The place rows a job's consent paints for a launch bound to `folder`.
fn scope_rows(folder: &str, cols: usize) -> Vec<String> {
    let card = approval(&scope_question(), Some(folder), SCREEN_ROWS);
    card.place_rows(room(cols))
}

#[test]
fn a_long_folder_keeps_its_own_name_at_sixty_columns() {
    let rows = scope_rows(LONG_FOLDER, 60);

    assert!(
        rows.iter().all(|row| cells(row) <= room(60)),
        "no row passes the line it was given: {rows:?}"
    );
    assert!(
        rows[0].starts_with("in …/"),
        "a folder too wide gives way at the front: {rows:?}"
    );
    assert!(
        rows[0].ends_with("repo"),
        "and keeps the folder's own name whole: {rows:?}"
    );
}

#[test]
fn a_long_folder_keeps_its_own_name_at_one_hundred_columns() {
    assert!(
        cells(LONG_FOLDER) + 3 > room(100),
        "the fixture folder is wider than the wide panel's line"
    );
    let rows = scope_rows(LONG_FOLDER, 100);

    assert!(
        rows.iter().all(|row| cells(row) <= room(100)),
        "no row passes the line it was given: {rows:?}"
    );
    assert!(
        rows[0].starts_with("in …/"),
        "a folder too wide gives way at the front: {rows:?}"
    );
    assert!(
        rows[0].ends_with("repo"),
        "and keeps the folder's own name whole: {rows:?}"
    );
}

#[test]
fn a_folder_under_the_home_directory_reads_as_tilde() {
    let card = approval(
        &scope_question(),
        Some("/home/me/projects/shop"),
        SCREEN_ROWS,
    );
    let place = card.place().expect("a job's consent names its folder");

    assert_eq!(
        place.rows_within(room(100), Some("/home/me")),
        [String::from("in ~/projects/shop")],
        "the home directory is the one prefix the line shortens for free"
    );
}

#[test]
fn the_lane_fact_moves_to_its_own_row_and_is_never_dropped() {
    // The command's own facts state the folder, so the place is the one the
    // operator's shell would run in.
    let question = Question {
        facts: format!("in: {LONG_FOLDER}\n$ cargo test"),
        ..shell_question()
    };
    let card = approval(&question, None, SCREEN_ROWS);
    let rows = card.place_rows(room(60));

    assert!(
        rows.join("\n").contains(NOT_SANDBOXED),
        "the safety fact is on the panel whichever row it lands on: {rows:?}"
    );
    assert_eq!(
        rows.len(),
        2,
        "a folder that fills the line leaves the next row for the lane: {rows:?}"
    );
    assert!(
        rows[0].ends_with("repo"),
        "and the folder's own name survives beside it: {rows:?}"
    );
    assert!(
        rows.iter().all(|row| cells(row) <= room(60)),
        "no row passes the line it was given: {rows:?}"
    );
}

#[test]
fn a_short_folder_keeps_the_lane_fact_on_the_same_row() {
    let card = approval(&shell_question(), Some("~/shop"), SCREEN_ROWS);

    assert_eq!(
        card.place_rows(room(100)),
        [format!("in ~/shop · {NOT_SANDBOXED}")],
        "a place with room to spare states the lane beside the folder"
    );
}

#[test]
fn a_name_wider_than_the_line_loses_its_middle_and_never_its_end() {
    let folder = "/private/var/folders/p8/T/saya-approval-panel-wide-87382";
    let card = approval(&scope_question(), Some(folder), SCREEN_ROWS);
    let rows = card.place_rows(12);

    assert!(
        rows[0].starts_with("in saya"),
        "the name keeps its start: {rows:?}"
    );
    assert!(
        rows[0].ends_with("7382"),
        "and its end — the middle is what gives way: {rows:?}"
    );
    assert!(rows[0].contains('…'), "the cut is marked: {rows:?}");
    assert!(
        rows.iter().all(|row| cells(row) <= 12),
        "no row passes the line it was given: {rows:?}"
    );
}
