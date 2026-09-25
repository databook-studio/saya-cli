//! Phase 8 packet 2: the unseen-rows count raised by the append path.
//!
//! Packet 1 froze a scrolled reader's window; these pin the other half — that
//! arrivals below the fold are counted (and only below the fold), and that
//! counting them never moves the window. Beside `push.rs`, which owns the one
//! append hook, per the testing standard.

use crate::interactive::tui::transcript::{BlockKind, Transcript};

/// Fills a transcript past one window and scrolls the reader up, then warms
/// the wrap cache so the next append sees a stored view — packet 1's
/// `scrolled_baseline` only compensates (and only counts) with a baseline.
fn scrolled_transcript() -> Transcript {
    let mut t = Transcript::new();
    for i in 0..8 {
        t.push(BlockKind::Assistant, format!("line {i}"));
    }
    t.scroll_up(2, 80, 4);
    assert!(
        !t.is_following_tail(),
        "precondition: the reader scrolled up"
    );
    let _ = t.view(80, 4);
    t
}

fn view_texts(t: &Transcript) -> Vec<String> {
    t.view(80, 4).iter().map(|row| row.text.clone()).collect()
}

#[test]
fn rows_appended_while_scrolled_up_raise_the_count() {
    let mut t = scrolled_transcript();
    assert_eq!(t.unseen_new_rows(), 0, "nothing has arrived below yet");
    t.push(BlockKind::Assistant, "landed below the fold");
    assert!(
        t.unseen_new_rows() > 0,
        "an append below a scrolled reader must raise the count"
    );
}

#[test]
fn rows_appended_at_the_tail_do_not_raise_the_count() {
    let mut t = Transcript::new();
    for i in 0..8 {
        t.push(BlockKind::Assistant, format!("line {i}"));
    }
    assert!(
        t.is_following_tail(),
        "precondition: watching the live tail"
    );
    let _ = t.view(80, 4);
    t.push(BlockKind::Assistant, "brand new");
    assert_eq!(
        t.unseen_new_rows(),
        0,
        "a count at the tail would be a lie: those rows are already visible"
    );
}

#[test]
fn scrolling_back_to_the_tail_clears_the_count() {
    let mut t = scrolled_transcript();
    t.push(BlockKind::Assistant, "landed below the fold");
    assert!(t.unseen_new_rows() > 0, "precondition: rows arrived below");
    t.scroll_down(usize::MAX);
    assert!(t.is_following_tail());
    assert_eq!(
        t.unseen_new_rows(),
        0,
        "reaching the tail by hand clears the count"
    );
}

#[test]
fn nothing_scrolls_the_user_automatically() {
    let mut t = scrolled_transcript();
    let before = view_texts(&t);
    t.push(BlockKind::Assistant, "landed below the fold");
    t.append_delta(BlockKind::Assistant, "\nand streaming on");
    assert_eq!(
        view_texts(&t),
        before,
        "counting new activity must not move the reader's window"
    );
    assert!(
        !t.is_following_tail(),
        "the reader stays where they were; returning is explicit"
    );
    assert!(
        t.unseen_new_rows() > 0,
        "the arrival is counted so the reader can see it happened"
    );
}
