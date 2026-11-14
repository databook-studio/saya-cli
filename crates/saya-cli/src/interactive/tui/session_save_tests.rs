//! The TUI save completion: a save of a warn-level record lands the
//! near-limit notice as one System block, and a save refused at the 4 MiB
//! cap shows the friendly refusal sentence instead of the raw store error.

use super::{poll_session_save, start_session_save};
use crate::interactive::session::lifecycle::capacity::limit_reached_text;
use crate::interactive::session::lifecycle::capacity::near_limit_text;
use crate::interactive::session::lifecycle::capacity_test_support::{
    state_with_turns, temp_store_root,
};
use crate::interactive::session::state::SessionState;
use crate::interactive::tui::transcript::BlockKind;
use crate::interactive::tui::types::App;
use crate::interactive::tui::ui_snapshot_tests::empty_app;
use saya_store::{FsSessionStore, SessionCapacityStatus};
use std::time::Duration;

/// Drains the background save until it completes, bounded.
fn drain(app: &mut App, store: &FsSessionStore, state: &mut SessionState) {
    for _ in 0..10_000 {
        if app.session_save.is_none() {
            return;
        }
        poll_session_save(app, store, state);
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("the background save never completed");
}

fn system_blocks(app: &App) -> Vec<String> {
    app.transcript
        .blocks()
        .iter()
        .filter(|block| block.kind == BlockKind::System)
        .map(|block| block.text.clone())
        .collect()
}

fn error_blocks(app: &App) -> Vec<String> {
    app.transcript
        .blocks()
        .iter()
        .filter(|block| block.kind == BlockKind::Error)
        .map(|block| block.text.clone())
        .collect()
}

#[test]
fn a_save_refused_at_the_limit_shows_the_friendly_sentence_not_the_raw_error() {
    let root = temp_store_root("tui-limit");
    let store = FsSessionStore::new(&root);
    let mut app = empty_app();
    let mut state = state_with_turns("cap-tui-limit", 2, 64);
    let oversized = state_with_turns("cap-tui-limit", 500, 11_500);
    assert_eq!(
        store
            .capacity_of(oversized.redacted())
            .expect("measures")
            .status,
        SessionCapacityStatus::OverCap,
        "the fixture is genuinely over the cap"
    );
    start_session_save(&mut app, &store, oversized.redacted());
    drain(&mut app, &store, &mut state);
    let errors = error_blocks(&app);
    assert_eq!(
        errors,
        vec![limit_reached_text("cap-tui-limit")],
        "exactly one Error block, the friendly sentence: {errors:?}"
    );
    assert!(
        !errors
            .iter()
            .any(|text| text.contains("value exceeds a store limit")),
        "the raw store error never shows: {errors:?}"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_near_limit_save_shows_the_system_notice_once_and_arms_the_flag() {
    let root = temp_store_root("tui-notice");
    let store = FsSessionStore::new(&root);
    let mut app = empty_app();
    let mut state = state_with_turns("cap-tui-notice", 300, 11_500);
    start_session_save(&mut app, &store, state.redacted());
    drain(&mut app, &store, &mut state);
    let capacity = store
        .stored_capacity("cap-tui-notice")
        .expect("metadata reads")
        .expect("the record is stored");
    assert_eq!(
        capacity.status,
        SessionCapacityStatus::Warning,
        "the fixture is between 3 MiB and the cap: {} B",
        capacity.used_bytes
    );
    assert_eq!(
        system_blocks(&app),
        vec![near_limit_text(capacity.used_bytes)],
        "one System block carrying the stored size"
    );
    assert!(state.capacity_warned, "the flag is set on the TUI path too");
    // A second warn-level save completes but says nothing more.
    start_session_save(&mut app, &store, state.redacted());
    drain(&mut app, &store, &mut state);
    assert_eq!(
        system_blocks(&app).len(),
        1,
        "the notice is once per session process: {:?}",
        system_blocks(&app)
    );
    let _ = std::fs::remove_dir_all(root);
}
