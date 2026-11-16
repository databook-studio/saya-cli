use super::{continuation_note, sticky_ceiling_after_cut};

#[test]
fn the_note_names_the_window_and_that_the_cut_was_discarded() {
    let note = continuation_note(302);
    assert!(note.contains("302s"), "{note}");
    assert!(note.contains("discarded"), "{note}");
    assert!(note.contains("Do not re-plan"), "{note}");
    assert!(
        !note.contains("one tool call") && !note.contains("smaller steps"),
        "a durable one-call order outlives compaction: {note}"
    );
}

/// A reasoning-only cut is not a measured output window. 85% of dumped
/// think pinned later turns to a still-large think budget (71,843 tokens in one
/// measured run).
#[test]
fn a_reasoning_only_cut_does_not_lower_the_later_ceiling() {
    assert_eq!(sticky_ceiling_after_cut(0, Some(393_216)), None);
    assert_eq!(sticky_ceiling_after_cut(0, None), None);
}

/// A gateway time-cut is a clock, including one that leaked a scrap of
/// visible text. Fitting 85% of that scrap and clamping up to 1,024 pinned
/// the rest of one measured run there (`sticky_ceiling_after_cut(10, …)`
/// returned `Some(1024)`). A large visible cut is the same clock.
#[test]
fn a_time_cut_with_visible_tokens_does_not_lower_the_ceiling() {
    assert_eq!(sticky_ceiling_after_cut(10, Some(393_216)), None);
    assert_eq!(sticky_ceiling_after_cut(100, Some(393_216)), None);
    assert_eq!(sticky_ceiling_after_cut(4_000, Some(4096)), None);
    assert_eq!(sticky_ceiling_after_cut(4_000, None), None);
    assert_eq!(sticky_ceiling_after_cut(62_000, Some(393_216)), None);
}
