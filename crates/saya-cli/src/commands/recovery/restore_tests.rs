//! Tests for `restore_change`: exact-byte restore through the guarded
//! workspace operations, guarded removal of created files, and every
//! refusal leaving the file byte-identical.

use std::fs;

use super::testing::*;
use super::*;
use saya_types::{CheckpointState, EditSource, RestoreEligibility};

#[test]
fn restore_puts_back_the_exact_before_bytes_and_consumes_the_checkpoint() {
    let sandbox = Sandbox::new("restore-replace");
    let v1: &[u8] = "first line\r\ncafé au lait\n".as_bytes();
    let v2: &[u8] = "first line\r\nlatte au lait\n".as_bytes();
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

    let outcome = restore_change(
        &sandbox.store,
        SESSION,
        Some(&sandbox.ws),
        record.id.as_str(),
    );
    assert_eq!(
        outcome,
        RestoreOutcome::Restored {
            id: record.id.as_str().to_owned(),
            path: "notes.txt".to_owned(),
            removed_checkpoint: true,
        }
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.txt")).expect("read"),
        v1,
        "the file holds the exact before bytes, including non-ASCII and CRLF"
    );
    assert!(
        sandbox
            .store
            .list_checkpoints(SESSION)
            .expect("list")
            .is_empty(),
        "the consumed checkpoint is deleted"
    );
    let view = list_changes(&sandbox.store, SESSION, Some(&sandbox.ws)).expect("list");
    assert!(
        view.items.is_empty(),
        "the restored edit is no longer listed"
    );
}

#[test]
fn restore_of_a_created_file_removes_it_and_keeps_the_parent_directory() {
    let sandbox = Sandbox::new("restore-create");
    let created: &[u8] = b"created\r\n";
    let record = record_commit(
        &sandbox,
        "sub/made.txt",
        EditSource::WorkspaceWrite,
        None,
        created,
        1_000,
    );
    fs::create_dir(sandbox.ws_root().join("sub")).expect("dir");
    fs::write(sandbox.ws_root().join("sub/made.txt"), created).expect("file");

    let outcome = restore_change(
        &sandbox.store,
        SESSION,
        Some(&sandbox.ws),
        record.id.as_str(),
    );
    assert_eq!(
        outcome,
        RestoreOutcome::Restored {
            id: record.id.as_str().to_owned(),
            path: "sub/made.txt".to_owned(),
            removed_checkpoint: true,
        }
    );
    assert!(
        !sandbox.ws_root().join("sub/made.txt").exists(),
        "the file the edit created is removed"
    );
    assert!(
        sandbox.ws_root().join("sub").is_dir(),
        "the empty parent directory is not removed"
    );
    assert!(
        sandbox
            .store
            .list_checkpoints(SESSION)
            .expect("list")
            .is_empty()
    );
}

#[test]
fn restore_refuses_when_the_file_changed_since_the_edit_and_touches_nothing() {
    let sandbox = Sandbox::new("restore-stale");
    let v1: &[u8] = b"before\n";
    let v2: &[u8] = b"after\n";
    let v3: &[u8] = b"newer work\n";
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
    fs::write(sandbox.ws_root().join("notes.txt"), v3).expect("newer change");

    let outcome = restore_change(
        &sandbox.store,
        SESSION,
        Some(&sandbox.ws),
        record.id.as_str(),
    );
    assert_eq!(
        outcome,
        RestoreOutcome::Refused(RestoreRefusal::NotEligible {
            status: StatusNow::Known(RestoreEligibility::FileChangedSince),
            path: "notes.txt".to_owned(),
        })
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.txt")).expect("read"),
        v3,
        "the refused restore leaves the file byte-identical"
    );
    assert_eq!(
        sandbox.store.list_checkpoints(SESSION).expect("list").len(),
        1,
        "the checkpoint is kept"
    );
}

#[test]
fn restore_refuses_a_same_length_change_to_the_file() {
    let sandbox = Sandbox::new("restore-race");
    let v1: &[u8] = b"before\n";
    let v2: &[u8] = b"after\n";
    let raced: &[u8] = b"raced\n";
    assert_eq!(v2.len(), raced.len(), "the race is a same-length rewrite");
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
    fs::write(sandbox.ws_root().join("notes.txt"), raced).expect("race");

    let outcome = restore_change(
        &sandbox.store,
        SESSION,
        Some(&sandbox.ws),
        record.id.as_str(),
    );
    assert_eq!(
        outcome,
        RestoreOutcome::Refused(RestoreRefusal::NotEligible {
            status: StatusNow::Known(RestoreEligibility::FileChangedSince),
            path: "notes.txt".to_owned(),
        })
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.txt")).expect("read"),
        raced
    );
}

#[test]
fn restore_refuses_when_the_file_is_missing() {
    let sandbox = Sandbox::new("restore-gone");
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

    let outcome = restore_change(
        &sandbox.store,
        SESSION,
        Some(&sandbox.ws),
        record.id.as_str(),
    );
    assert_eq!(
        outcome,
        RestoreOutcome::Refused(RestoreRefusal::NotEligible {
            status: StatusNow::Known(RestoreEligibility::FileMissing),
            path: "notes.txt".to_owned(),
        })
    );
    assert_eq!(
        sandbox.store.list_checkpoints(SESSION).expect("list").len(),
        1
    );
}

#[test]
fn restore_refuses_a_tampered_before_image_and_touches_nothing() {
    let sandbox = Sandbox::new("restore-tamper");
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
    let image = sandbox
        .checkpoints_dir()
        .join(format!("{}.before", record.id.as_str()));
    fs::write(&image, b"tampered bytes").expect("tamper");

    let outcome = restore_change(
        &sandbox.store,
        SESSION,
        Some(&sandbox.ws),
        record.id.as_str(),
    );
    assert_eq!(
        outcome,
        RestoreOutcome::Refused(RestoreRefusal::BeforeImageCorrupt)
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.txt")).expect("read"),
        v2,
        "the refused restore leaves the file byte-identical"
    );
    assert_eq!(
        sandbox.store.list_checkpoints(SESSION).expect("list").len(),
        1
    );
}

#[test]
fn restore_refuses_unknown_and_malformed_ids() {
    let sandbox = Sandbox::new("restore-unknown");
    let unknown = format!("ck-{}", "ab".repeat(16));
    assert_eq!(
        restore_change(&sandbox.store, SESSION, Some(&sandbox.ws), &unknown),
        RestoreOutcome::Refused(RestoreRefusal::NotFound)
    );
    assert_eq!(
        restore_change(
            &sandbox.store,
            SESSION,
            Some(&sandbox.ws),
            "not-a-checkpoint"
        ),
        RestoreOutcome::Refused(RestoreRefusal::NotFound)
    );
    assert_eq!(
        restore_change(&sandbox.store, SESSION, Some(&sandbox.ws), ""),
        RestoreOutcome::Refused(RestoreRefusal::NotFound)
    );
}

#[test]
fn restore_without_a_workspace_refuses() {
    let sandbox = Sandbox::new("restore-nows");
    let record = record_commit(
        &sandbox,
        "notes.txt",
        EditSource::WorkspaceWrite,
        None,
        b"x",
        1_000,
    );
    assert_eq!(
        restore_change(&sandbox.store, SESSION, None, record.id.as_str()),
        RestoreOutcome::Refused(RestoreRefusal::NoWorkspace)
    );
}

#[test]
fn restore_refuses_records_that_never_committed() {
    let sandbox = Sandbox::new("restore-uncommitted");
    let prepared = record_prepared(&sandbox, "notes.txt", Some(b"old"), 1_000);
    assert_eq!(
        restore_change(
            &sandbox.store,
            SESSION,
            Some(&sandbox.ws),
            prepared.id.as_str()
        ),
        RestoreOutcome::Refused(RestoreRefusal::NotEligible {
            status: StatusNow::Known(RestoreEligibility::NotCommitted),
            path: "notes.txt".to_owned(),
        })
    );
    sandbox
        .store
        .mark_checkpoint_uncertain(SESSION, prepared.id.as_str(), 2_000)
        .expect("mark");
    assert_eq!(
        restore_change(
            &sandbox.store,
            SESSION,
            Some(&sandbox.ws),
            prepared.id.as_str()
        ),
        RestoreOutcome::Refused(RestoreRefusal::NotEligible {
            status: StatusNow::Known(RestoreEligibility::EffectUncertain),
            path: "notes.txt".to_owned(),
        })
    );
    let records = sandbox.store.list_checkpoints(SESSION).expect("list");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].state, CheckpointState::Uncertain);
}

#[cfg(unix)]
#[test]
fn restore_reports_the_harness_refusal_for_a_symlinked_path_component() {
    let sandbox = Sandbox::new("restore-symlink");
    let outside = sandbox.outer.join("outside");
    fs::create_dir_all(&outside).expect("outside dir");
    let target: &[u8] = b"inside bytes\n";
    fs::write(outside.join("inside.txt"), target).expect("target");
    std::os::unix::fs::symlink(&outside, sandbox.ws_root().join("link")).expect("link");
    let record = record_commit(
        &sandbox,
        "link/inside.txt",
        EditSource::WorkspaceWrite,
        None,
        target,
        1_000,
    );

    let outcome = restore_change(
        &sandbox.store,
        SESSION,
        Some(&sandbox.ws),
        record.id.as_str(),
    );
    match &outcome {
        RestoreOutcome::Refused(RestoreRefusal::Failed(error)) => {
            assert!(
                error.contains("symlink"),
                "the refusal names the link: {error}"
            );
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    assert_eq!(
        fs::read(outside.join("inside.txt")).expect("read"),
        target,
        "the link target is untouched"
    );
    assert_eq!(
        sandbox.store.list_checkpoints(SESSION).expect("list").len(),
        1,
        "the checkpoint is kept"
    );
}

#[cfg(unix)]
#[test]
fn a_failed_checkpoint_deletion_still_reports_the_file_restored() {
    use std::os::unix::fs::PermissionsExt;

    let sandbox = Sandbox::new("restore-keep");
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
    let dir = sandbox.checkpoints_dir();
    let original = fs::metadata(&dir).expect("meta").permissions();
    let readonly = {
        let mut locked = original.clone();
        locked.set_mode(0o555);
        locked
    };
    fs::set_permissions(&dir, readonly).expect("lock");
    let outcome = restore_change(
        &sandbox.store,
        SESSION,
        Some(&sandbox.ws),
        record.id.as_str(),
    );
    fs::set_permissions(&dir, original).expect("unlock");

    assert_eq!(
        outcome,
        RestoreOutcome::Restored {
            id: record.id.as_str().to_owned(),
            path: "notes.txt".to_owned(),
            removed_checkpoint: false,
        }
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.txt")).expect("read"),
        v1,
        "the file IS restored even though the checkpoint remains"
    );
    assert_eq!(
        sandbox.store.list_checkpoints(SESSION).expect("list").len(),
        1
    );
}
