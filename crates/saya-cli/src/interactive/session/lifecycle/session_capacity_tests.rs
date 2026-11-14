//! The capacity seam: the pinned sentences, the one-time notice a successful
//! save can fire, and the friendly refusal when a save is refused at the
//! 4 MiB cap — with the real store and the real serialization accounting.

use super::super::rotate::rotate_state;
use super::{
    SaveFailure, limit_reached_text, near_limit_text, notice_after_save, save_with_explanation,
};
use crate::interactive::session::continuation::compaction::manual::{
    apply_with_continuation, plan as compact_plan,
};
use crate::interactive::session::lifecycle::capacity_test_support::{
    state_with_turns, temp_store_root,
};
use crate::interactive::session::lifecycle::resume::block_on;
use crate::interactive::session::state::SessionState;
use saya_store::{FsSessionStore, SessionCapacityStatus, SessionStore};

/// 3.14 MiB in bytes: the pinned one-decimal rounding case.
const THREE_POINT_ONE_FOUR_MIB: usize = 3_292_529;

#[test]
fn the_near_limit_sentence_is_pinned_with_one_decimal_place() {
    assert_eq!(
        near_limit_text(THREE_POINT_ONE_FOUR_MIB),
        "This session's saved record is 3.1 MiB of its 4 MiB limit. \
         Run /compact, then /rotate to continue in a smaller successor session."
    );
    assert!(
        near_limit_text(3_355_443)
            .starts_with("This session's saved record is 3.2 MiB of its 4 MiB limit."),
        "3.2 MiB renders with one decimal place: {}",
        near_limit_text(3_355_443)
    );
}

#[test]
fn the_limit_refusal_sentence_is_pinned_and_names_the_resume_path() {
    assert_eq!(
        limit_reached_text("cap-9"),
        "This session's record has reached its 4 MiB limit, so the latest turn could not be saved. \
         Run /compact, then /rotate to continue in a smaller successor session; \
         /resume cap-9 restores the last saved state."
    );
}

#[test]
fn the_capacity_flag_starts_armed_and_stays_out_of_persisted_sessions() {
    let fresh = SessionState::new("s1", None, "m");
    assert!(
        !fresh.capacity_warned,
        "a fresh session starts with the notice armed (unfired)"
    );
    let json = serde_json::to_string(&fresh).expect("serializes");
    assert!(
        !json.contains("capacity_warned"),
        "the flag must stay out of persisted sessions: {json}"
    );
}

#[test]
fn a_record_below_the_warning_level_says_nothing() {
    let root = temp_store_root("below-warn");
    let store = FsSessionStore::new(&root);
    let mut state = state_with_turns("cap-below", 2, 64);
    block_on(store.save(state.redacted())).expect("the small record saves");
    assert_eq!(
        notice_after_save(&store, &mut state),
        None,
        "below 3 MiB there is no notice"
    );
    assert!(!state.capacity_warned, "the one-time flag stays unarmed");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_record_at_or_above_the_warning_level_says_the_notice_exactly_once() {
    let root = temp_store_root("warn-once");
    let store = FsSessionStore::new(&root);
    let mut state = state_with_turns("cap-warn", 300, 11_500);
    block_on(store.save(state.redacted())).expect("the warn-level record saves");
    let capacity = store
        .stored_capacity("cap-warn")
        .expect("metadata reads")
        .expect("the record is stored");
    assert_eq!(
        capacity.status,
        SessionCapacityStatus::Warning,
        "the fixture is between 3 MiB and the cap: {} B",
        capacity.used_bytes
    );
    let first = notice_after_save(&store, &mut state);
    assert_eq!(
        first.as_deref(),
        Some(near_limit_text(capacity.used_bytes).as_str()),
        "the notice carries the stored size"
    );
    assert!(state.capacity_warned, "the one-time flag is set");
    assert_eq!(
        notice_after_save(&store, &mut state),
        None,
        "one notice per session process: the next save says nothing"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_missing_record_says_nothing() {
    let root = temp_store_root("missing");
    let store = FsSessionStore::new(&root);
    let mut state = state_with_turns("cap-absent", 2, 64);
    assert_eq!(
        notice_after_save(&store, &mut state),
        None,
        "no stored record: no notice"
    );
    assert!(!state.capacity_warned);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_save_refused_at_the_limit_explains_and_leaves_the_record_untouched() {
    let root = temp_store_root("limit-refusal");
    let store = FsSessionStore::new(&root);
    let saved = state_with_turns("cap-limit", 2, 64);
    block_on(store.save(saved.redacted())).expect("the last good record saves");
    let before = std::fs::read(root.join("cap-limit.json")).expect("the record reads");
    let oversized = state_with_turns("cap-limit", 500, 11_500);
    assert_eq!(
        store
            .capacity_of(oversized.redacted())
            .expect("measures")
            .status,
        SessionCapacityStatus::OverCap,
        "the fixture is genuinely over the cap"
    );
    let failure =
        save_with_explanation(&store, &oversized).expect_err("the over-cap save is refused");
    match failure {
        SaveFailure::LimitReached(text) => {
            assert_eq!(
                text,
                limit_reached_text("cap-limit"),
                "the friendly refusal replaces the raw store error"
            );
        }
        SaveFailure::Other(error) => {
            panic!("the limit refusal must be worded, not raw: {error}")
        }
    }
    let after = std::fs::read(root.join("cap-limit.json")).expect("the record reads");
    assert_eq!(
        before, after,
        "the last good record is byte-identical after the refusal"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_save_under_the_cap_saves_plainly() {
    let root = temp_store_root("plain-save");
    let store = FsSessionStore::new(&root);
    let state = state_with_turns("cap-ok", 2, 64);
    assert!(
        save_with_explanation(&store, &state).is_ok(),
        "an ordinary save is not a failure"
    );
    assert!(root.join("cap-ok.json").exists(), "the record landed");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_rotation_successor_starts_with_the_capacity_notice_armed_again() {
    let mut parent = state_with_turns("cap-rot-parent", 6, 64);
    let history = parent.provider_history();
    let plan = compact_plan(&parent.turns, &history).expect("a long history plans");
    apply_with_continuation(&mut parent, &plan, "older turns discussed orders", &|_| {
        None
    })
    .expect("the summary applies");
    parent.capacity_warned = true;
    let successor =
        rotate_state(&parent, "cap-rot-child", "rot-1").expect("a compacted parent rotates");
    assert!(
        !successor.capacity_warned,
        "a successor is a new session process: the notice re-arms"
    );
}
