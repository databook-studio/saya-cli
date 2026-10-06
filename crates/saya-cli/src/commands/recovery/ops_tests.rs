//! Tests for `list_changes`: every recorded edit with its current restore
//! status against the file on disk, session quota usage, and the bounded
//! per-call refresh.

use std::fs;

use super::testing::*;
use super::*;
use saya_types::{
    CheckpointState, EditSource, MAX_CHECKPOINT_SESSION_BYTES, MAX_CHECKPOINTS_PER_SESSION,
    RestoreEligibility,
};

#[test]
fn an_empty_session_lists_no_edits() {
    let sandbox = Sandbox::new("list-empty");
    let view = list_changes(&sandbox.store, SESSION, Some(&sandbox.ws)).expect("list");
    assert!(view.items.is_empty());
    assert_eq!(view.usage.edits, 0);
    assert_eq!(view.usage.retained_bytes, 0);
    assert_eq!(view.max_edits, MAX_CHECKPOINTS_PER_SESSION);
    assert_eq!(view.max_bytes, MAX_CHECKPOINT_SESSION_BYTES);
    assert_eq!(
        render_changes(&view, 0),
        "No recorded edits. Recovery records file edits only while it is on (opt-in); \
         it covers workspace_write/workspace_edit — never commands, downloads, scratch \
         or database effects."
    );
}

#[test]
fn two_edits_of_one_file_list_eligible_and_changed_since() {
    let sandbox = Sandbox::new("list-two");
    let v1: &[u8] = b"one\r\ntwo\n";
    let v2: &[u8] = b"one\r\nTWO\n";
    let v3: &[u8] = b"ONE\r\nTWO\n";
    fs::write(sandbox.ws_root().join("notes.txt"), v1).expect("seed");
    let first = record_commit(
        &sandbox,
        "notes.txt",
        EditSource::WorkspaceEdit,
        Some(v1),
        v2,
        1_000,
    );
    fs::write(sandbox.ws_root().join("notes.txt"), v2).expect("edit");
    let second = record_commit(
        &sandbox,
        "notes.txt",
        EditSource::WorkspaceWrite,
        Some(v2),
        v3,
        2_000,
    );
    fs::write(sandbox.ws_root().join("notes.txt"), v3).expect("edit");

    let view = list_changes(&sandbox.store, SESSION, Some(&sandbox.ws)).expect("list");
    assert_eq!(view.items.len(), 2);
    let earlier = &view.items[0];
    assert_eq!(earlier.id, first.id.as_str());
    assert_eq!(earlier.path, "notes.txt");
    assert_eq!(earlier.source, EditSource::WorkspaceEdit);
    assert_eq!(earlier.state, CheckpointState::Committed);
    assert_eq!(
        earlier.before,
        BeforeSummary::Present {
            size: v1.len() as u64
        }
    );
    let earlier_after = earlier.after.as_ref().expect("committed record");
    assert_eq!(earlier_after.sha256, digest(v2));
    assert_eq!(earlier_after.size, v2.len() as u64);
    assert_eq!(earlier.created_unix_ms, 1_000);
    assert_eq!(
        earlier.restore,
        StatusNow::Known(RestoreEligibility::FileChangedSince)
    );

    let latest = &view.items[1];
    assert_eq!(latest.id, second.id.as_str());
    assert_eq!(latest.source, EditSource::WorkspaceWrite);
    assert_eq!(
        latest.before,
        BeforeSummary::Present {
            size: v2.len() as u64
        }
    );
    assert_eq!(
        latest.restore,
        StatusNow::Known(RestoreEligibility::Eligible)
    );
}

#[test]
fn a_deleted_file_lists_as_missing() {
    let sandbox = Sandbox::new("list-missing");
    let v1: &[u8] = b"before\n";
    let v2: &[u8] = b"after\n";
    fs::write(sandbox.ws_root().join("notes.txt"), v1).expect("seed");
    let record = record_commit(
        &sandbox,
        "notes.txt",
        EditSource::WorkspaceEdit,
        Some(v1),
        v2,
        1_000,
    );
    fs::write(sandbox.ws_root().join("notes.txt"), v2).expect("edit");
    fs::remove_file(sandbox.ws_root().join("notes.txt")).expect("delete");

    let view = list_changes(&sandbox.store, SESSION, Some(&sandbox.ws)).expect("list");
    assert_eq!(view.items.len(), 1);
    assert_eq!(view.items[0].id, record.id.as_str());
    assert_eq!(
        view.items[0].restore,
        StatusNow::Known(RestoreEligibility::FileMissing)
    );
}

#[test]
fn prepared_and_uncertain_records_list_their_state_without_a_refresh() {
    let sandbox = Sandbox::new("list-states");
    let prepared = record_prepared(&sandbox, "prepared.txt", Some(b"old"), 1_000);
    let uncertain = record_prepared(&sandbox, "uncertain.txt", Some(b"old"), 2_000);
    sandbox
        .store
        .mark_checkpoint_uncertain(SESSION, uncertain.id.as_str(), 2_000)
        .expect("mark");

    let view = list_changes(&sandbox.store, SESSION, Some(&sandbox.ws)).expect("list");
    assert_eq!(view.items.len(), 2);
    assert_eq!(view.items[0].id, prepared.id.as_str());
    assert_eq!(view.items[0].state, CheckpointState::Prepared);
    assert_eq!(
        view.items[0].restore,
        StatusNow::Known(RestoreEligibility::NotCommitted)
    );
    assert_eq!(view.items[1].id, uncertain.id.as_str());
    assert_eq!(view.items[1].state, CheckpointState::Uncertain);
    assert_eq!(
        view.items[1].restore,
        StatusNow::Known(RestoreEligibility::EffectUncertain)
    );
}

#[test]
fn usage_counts_edits_and_retained_before_bytes() {
    let sandbox = Sandbox::new("list-usage");
    let v1: &[u8] = b"0123456789";
    let v2: &[u8] = b"0123456789ABCDEF";
    fs::write(sandbox.ws_root().join("a.txt"), b"seed a").expect("seed");
    record_commit(
        &sandbox,
        "a.txt",
        EditSource::WorkspaceEdit,
        Some(v1),
        v2,
        1_000,
    );
    fs::write(sandbox.ws_root().join("a.txt"), v2).expect("edit");
    fs::write(sandbox.ws_root().join("b.txt"), b"seed b").expect("seed");
    record_commit(
        &sandbox,
        "b.txt",
        EditSource::WorkspaceWrite,
        Some(v2),
        v1,
        2_000,
    );
    fs::write(sandbox.ws_root().join("b.txt"), v1).expect("edit");

    let view = list_changes(&sandbox.store, SESSION, Some(&sandbox.ws)).expect("list");
    assert_eq!(view.usage.edits, 2);
    assert_eq!(view.usage.retained_bytes, 26);
}

#[test]
fn only_the_first_32_committed_records_are_refreshed() {
    let sandbox = Sandbox::new("list-bound");
    for i in 0..40usize {
        let name = format!("file{i:02}.txt");
        let before = format!("old {i}\n").into_bytes();
        let after = format!("new {i}\n").into_bytes();
        fs::write(sandbox.ws_root().join(&name), &before).expect("seed");
        record_commit(
            &sandbox,
            &name,
            EditSource::WorkspaceEdit,
            Some(&before),
            &after,
            1_000 + i as i64,
        );
        fs::write(sandbox.ws_root().join(&name), &after).expect("edit");
    }

    let view = list_changes(&sandbox.store, SESSION, Some(&sandbox.ws)).expect("list");
    assert_eq!(view.items.len(), 40);
    for item in view.items.iter().take(32) {
        assert_eq!(
            item.restore,
            StatusNow::Known(RestoreEligibility::Eligible),
            "the oldest 32 are refreshed: {}",
            item.path
        );
    }
    for item in view.items.iter().skip(32) {
        assert_eq!(
            item.restore,
            StatusNow::NotChecked,
            "beyond the refresh bound: {}",
            item.path
        );
    }
}

#[test]
fn a_file_that_cannot_be_read_lists_as_unreadable() {
    let sandbox = Sandbox::new("list-unreadable");
    fs::create_dir(sandbox.ws_root().join("adirectory")).expect("dir");
    let record = record_commit(
        &sandbox,
        "adirectory",
        EditSource::WorkspaceEdit,
        Some(b"x"),
        b"y",
        1_000,
    );

    let view = list_changes(&sandbox.store, SESSION, Some(&sandbox.ws)).expect("list");
    assert_eq!(view.items.len(), 1);
    assert_eq!(view.items[0].id, record.id.as_str());
    assert_eq!(view.items[0].restore, StatusNow::Unreadable);
}
