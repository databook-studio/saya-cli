//! Twin of `commands/recovery/session_tests/*`, deleted with the old turn
//! body: the `/recovery` operation layer and the record-id resolver.
//!
//! `slash/tests/notes_recovery.rs` pins the *parsing* of every sub-word; these
//! pin what the parsed command does to the session's own handle and store, and
//! what a typed id resolves to before the store is asked.

use saya_types::{CheckpointState, EditSource};

use super::session::resolve_id;
use super::testing::{SESSION, Sandbox, record_commit};
use super::*;
use crate::interactive::session::lifecycle::recovery::RecoveryHandle;

/// The status line of an untouched session, byte for byte.
const STATUS_OFF: &str =
    "Recovery is off. 0 recorded edit(s) this session (0 KiB of 64 MiB, 0 of 128).";

/// The unbound refusal, byte for byte.
const NO_WORKSPACE: &str = "No workspace is bound, so there are no file edits to record. Launch \
                            inside a git worktree or pass --workspace <dir>.";

/// One recorded edit as `resolve_id` sees it.
fn item(id: &str) -> ChangeItem {
    ChangeItem {
        id: id.to_owned(),
        path: "notes.txt".to_owned(),
        source: EditSource::WorkspaceEdit,
        state: CheckpointState::Committed,
        before: BeforeSummary::Absent,
        after: None,
        created_unix_ms: 0,
        restore: StatusNow::Unreadable,
    }
}

/// The session's own handle over the sandbox's session store.
fn handle(sandbox: &Sandbox) -> RecoveryHandle {
    RecoveryHandle::new(&sandbox.outer.join("sessions"), SESSION)
}

#[test]
fn the_full_id_and_a_unique_prefix_resolve_to_the_one_edit() {
    let items = [item("ck-aaaaaa1111"), item("ck-bbbbbb2222")];
    let full = "ck-aaaaaa1111";
    assert_eq!(resolve_id(&items, full).expect("full id"), full);
    assert_eq!(
        resolve_id(&items, "ck-aaaaaa").expect("unique prefix"),
        full,
        "a unique prefix names the one edit"
    );
    let prefixed = [item("ck-aaaaaa1111"), item("ck-bbbbbb2222")];
    assert_eq!(
        resolve_id(&prefixed, "ck-bbbbbb").expect("unique prefix"),
        "ck-bbbbbb2222"
    );
}

#[test]
fn an_ambiguous_prefix_names_the_problem() {
    let items = [item("ck-aaaaaa1111"), item("ck-aaaaaa2222")];
    let error = resolve_id(&items, "ck-aaaaaa").expect_err("two matches cannot resolve");
    assert!(
        error.contains("matches 2 recorded edits"),
        "the ambiguity is named: {error}"
    );
}

#[test]
fn a_too_short_prefix_is_refused() {
    let items = [item("ck-aaaaaa1111")];
    let error = resolve_id(&items, "ck-abc").expect_err("a short prefix cannot identify one");
    assert!(
        error.contains("too short"),
        "the short prefix is named: {error}"
    );
}

#[test]
fn an_unknown_or_malformed_id_is_refused_without_reaching_the_store() {
    let items = [item("ck-aaaaaa1111")];
    let unknown = resolve_id(&items, "ck-zzzzzz").expect_err("no match cannot resolve");
    assert!(
        unknown.contains("matches no recorded edit"),
        "the unknown prefix is named: {unknown}"
    );
    let malformed = resolve_id(&items, "not-a-checkpoint").expect_err("no ck- shape");
    assert!(
        malformed.contains("ck-"),
        "the malformed id names the id shape: {malformed}"
    );
    // The resolver is pure over the listing: refusing never touches the items.
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "ck-aaaaaa1111");
}

#[test]
fn status_reports_the_flag_and_the_quota_usage() {
    let sandbox = Sandbox::new("status");
    let handle = handle(&sandbox);
    let (ok, text) = run_recovery(RecoveryCommand::Status, &handle, Some(&sandbox.ws), 0);
    assert!(ok, "{text}");
    assert_eq!(text, STATUS_OFF, "{text}");
}

#[test]
fn status_without_a_workspace_says_so() {
    let sandbox = Sandbox::new("status-unbound");
    let handle = handle(&sandbox);
    let (ok, text) = run_recovery(RecoveryCommand::Status, &handle, None, 0);
    assert!(ok, "{text}");
    assert!(
        text.contains("Recovery is off. 0 recorded edit(s)"),
        "{text}"
    );
    assert!(text.contains("No workspace is bound."), "{text}");
}

#[test]
fn on_pins_the_retention_text_and_flips_the_session_flag() {
    let sandbox = Sandbox::new("on");
    let handle = handle(&sandbox);
    assert!(
        !handle.is_enabled(),
        "recovery is opt-in: off at acquisition"
    );
    let (ok, text) = run_recovery(RecoveryCommand::On, &handle, Some(&sandbox.ws), 0);
    assert!(ok, "{text}");
    assert!(text.contains("Recovery is on."), "{text}");
    for retention in ["128 edits, 64 MiB and 8 MiB per file", "/sessions delete"] {
        assert!(
            text.contains(retention),
            "the retention text states {retention}: {text}"
        );
    }
    assert!(handle.is_enabled(), "the session flag flips on");
}

#[test]
fn on_twice_says_already_on() {
    let sandbox = Sandbox::new("on-twice");
    let handle = handle(&sandbox);
    let (ok, _) = run_recovery(RecoveryCommand::On, &handle, Some(&sandbox.ws), 0);
    assert!(ok);
    let (ok, text) = run_recovery(RecoveryCommand::On, &handle, Some(&sandbox.ws), 0);
    assert!(ok, "{text}");
    assert_eq!(text, "Recovery is already on.", "{text}");
}

#[test]
fn on_without_a_workspace_refuses_and_the_flag_stays_off() {
    let sandbox = Sandbox::new("on-unbound");
    let handle = handle(&sandbox);
    let (ok, text) = run_recovery(RecoveryCommand::On, &handle, None, 0);
    assert!(!ok, "{text}");
    assert_eq!(text, NO_WORKSPACE, "{text}");
    assert!(!handle.is_enabled(), "a refused on leaves the flag off");
}

#[test]
fn off_flips_the_flag_and_keeps_what_was_recorded() {
    let sandbox = Sandbox::new("off");
    let _record = record_commit(
        &sandbox,
        "notes.txt",
        EditSource::WorkspaceEdit,
        None,
        b"written\n",
        1_000,
    );
    let handle = handle(&sandbox);
    let (ok, _) = run_recovery(RecoveryCommand::On, &handle, Some(&sandbox.ws), 0);
    assert!(ok);
    let (ok, text) = run_recovery(RecoveryCommand::Off, &handle, Some(&sandbox.ws), 0);
    assert!(ok, "{text}");
    assert_eq!(text, "Recovery is off.", "{text}");
    assert!(!handle.is_enabled(), "the session flag flips off");
    let view = list_changes(&sandbox.store, SESSION, Some(&sandbox.ws)).expect("list");
    assert_eq!(view.items.len(), 1, "turning off keeps what was recorded");
}
