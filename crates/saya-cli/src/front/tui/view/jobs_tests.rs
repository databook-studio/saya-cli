//! The jobs drawer's value, read as words: `02` §8's three rows with their own
//! facts, the selection walking them, the confirm that asks before a cancel, and
//! the detail `↵` shows while a job's transcript is not part of this surface.

use super::jobs::{CONFIRM_ASK, DETAIL_UNIT, Drawer, State, WAITING_FOR_OPERATOR};
use super::jobs_fixture::{drawer, finished, paused, running};

#[test]
fn a_running_job_reads_its_age_and_its_helpers() {
    assert_eq!(
        running().facts(),
        "running 3m · 2 helpers",
        "02 §8: ● fix-flaky running 3m · 2 helpers"
    );
}

#[test]
fn a_running_job_with_one_helper_says_one_helper() {
    let mut job = running();
    job.helpers = 1;
    assert_eq!(job.facts(), "running 3m · 1 helper");
}

#[test]
fn a_paused_job_reads_the_reason_it_paused() {
    assert_eq!(
        paused().facts(),
        "paused: no progress",
        "02 §8: ⏸ docs-pass paused: no progress"
    );
}

#[test]
fn a_finished_job_reads_what_it_ended_with() {
    assert_eq!(
        finished().facts(),
        "finished · 4 files changed",
        "02 §8: ✓ bump-deps finished · 4 files changed"
    );
}

#[test]
fn a_job_waiting_on_a_call_says_so() {
    let mut job = running();
    job.summary = String::from(WAITING_FOR_OPERATOR);
    assert_eq!(job.call, None);
    job.call = Some(7);
    assert!(
        job.facts().contains(WAITING_FOR_OPERATOR),
        "a job whose call is on the card says what it waits for: {}",
        job.facts()
    );
}

#[test]
fn each_state_carries_the_glyph_of_its_row() {
    use crate::front::tui::theme::{Charset, glyph};
    assert_eq!(glyph(State::Running.mark(), Charset::Utf8), "●");
    assert_eq!(glyph(State::Paused.mark(), Charset::Utf8), "⏸");
    assert_eq!(glyph(State::Finished.mark(), Charset::Utf8), "✓");
    assert_eq!(State::Running.word(), "running");
    assert_eq!(State::Paused.word(), "paused");
    assert_eq!(State::Finished.word(), "finished");
}

#[test]
fn a_rows_line_is_its_mark_its_id_and_its_facts() {
    let drawer = drawer();
    assert_eq!(
        drawer.line(&running()),
        "● fix-flaky  running 3m · 2 helpers"
    );
    assert_eq!(drawer.line(&paused()), "⏸ docs-pass  paused: no progress");
    assert_eq!(
        drawer.line(&finished()),
        "✓ bump-deps  finished · 4 files changed"
    );
}

#[test]
fn the_drawer_walks_its_rows_and_stops_at_both_ends() {
    let mut drawer = drawer();
    assert_eq!(drawer.selected(), 0, "the drawer opens on the first row");
    drawer.up();
    assert_eq!(drawer.selected(), 0, "up on the first row stays put");
    drawer.down();
    drawer.down();
    assert_eq!(drawer.selected(), 2);
    assert_eq!(
        drawer.selected_row().map(|row| row.id.as_str()),
        Some("bump-deps")
    );
    drawer.down();
    assert_eq!(drawer.selected(), 2, "down on the last row stays put");
    drawer.up();
    assert_eq!(drawer.selected(), 1);
}

#[test]
fn an_empty_drawer_has_no_row_to_name() {
    let drawer = Drawer::default();
    assert!(drawer.is_empty());
    assert!(drawer.selected_row().is_none());
    assert!(drawer.ask().is_none());
    assert!(drawer.detail().is_none());
}

#[test]
fn x_asks_before_it_cancels() {
    let mut drawer = drawer();
    assert!(!drawer.is_confirming());
    drawer.ask_cancel();
    assert!(drawer.is_confirming(), "x asks first, exactly like T12's d");
    assert_eq!(
        drawer.ask().as_deref(),
        Some("cancel fix-flaky? y/n"),
        "the ask names the row it would cancel"
    );
    assert_eq!(
        drawer.answer(true),
        Some(String::from("fix-flaky")),
        "y cancels"
    );
    assert!(!drawer.is_confirming(), "and the ask is over");
    assert_eq!(CONFIRM_ASK, "cancel");
}

#[test]
fn n_withdraws_the_ask_and_cancels_nothing() {
    let mut drawer = drawer();
    drawer.down();
    drawer.ask_cancel();
    assert_eq!(drawer.answer(false), None, "n cancels nothing");
    assert!(!drawer.is_confirming());
}

#[test]
fn esc_withdraws_the_ask_before_it_does_anything_else() {
    let mut drawer = drawer();
    drawer.ask_cancel();
    drawer.escape();
    assert!(!drawer.is_confirming(), "Esc withdraws the ask");
}

#[test]
fn the_confirm_owns_the_keyboard_while_it_asks() {
    let mut drawer = drawer();
    drawer.ask_cancel();
    drawer.ask_cancel();
    drawer.down();
    assert_eq!(drawer.selected(), 0, "the rows do not walk under the ask");
}

#[test]
fn the_detail_is_the_jobs_summary_line_and_says_what_is_missing() {
    let detail = drawer()
        .detail()
        .expect("a selected row's detail is the row and the unit that is missing");
    assert_eq!(
        detail.len(),
        2,
        "the summary line and the sentence: {detail:?}"
    );
    assert!(
        detail[0].contains("fix-flaky") && detail[0].contains("running 3m"),
        "the job's own summary line: {detail:?}"
    );
    assert_eq!(detail[1], DETAIL_UNIT);
    assert!(
        DETAIL_UNIT.contains("job transcript"),
        "↵ says which unit a job's own detail needs: {DETAIL_UNIT}"
    );
}
