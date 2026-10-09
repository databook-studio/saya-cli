#[test]
fn status_reports_the_flag_and_the_quota_usage() {
    let sandbox = Sandbox::new("recovery-status");
    let handle = handle_for(&sandbox);
    let (ok, text) = run_recovery(RecoveryCommand::Status, &handle, Some(&sandbox.ws), NOW);
    assert!(ok, "{text}");
    assert_eq!(
        text, "Recovery is off. 0 recorded edit(s) this session (0 KiB of 64 MiB, 0 of 128).",
        "{text}"
    );

    // Two committed edits through the real store lifecycle: the counts and
    // the quota line follow, and the off-state hint says they stay usable.
    record_commit(
        &sandbox,
        "notes.txt",
        EditSource::WorkspaceEdit,
        Some(b"v1\n"),
        b"v2\n",
        NOW,
    );
    record_commit(
        &sandbox,
        "other.txt",
        EditSource::WorkspaceWrite,
        Some(b"a\n"),
        b"b\n",
        NOW,
    );
    let (ok, text) = run_recovery(RecoveryCommand::Status, &handle, Some(&sandbox.ws), NOW);
    assert!(ok, "{text}");
    assert!(
        text.starts_with("Recovery is off. 2 recorded edit(s) this session ("),
        "{text}"
    );
    assert!(text.contains(" of 64 MiB, 2 of 128)."), "{text}");
    assert!(
        text.contains(" Recorded edits stay available: /recovery list."),
        "{text}"
    );

    // On with the same records: the hint belongs to the off state only.
    handle.set_enabled(true);
    let (ok, text) = run_recovery(RecoveryCommand::Status, &handle, Some(&sandbox.ws), NOW);
    assert!(ok, "{text}");
    assert!(
        text.starts_with("Recovery is on. 2 recorded edit(s)"),
        "{text}"
    );
    assert!(!text.contains("stay available"), "{text}");
}

#[test]
fn status_without_a_workspace_says_so() {
    let sandbox = Sandbox::new("recovery-status-unbound");
    let handle = handle_for(&sandbox);
    let (ok, text) = run_recovery(RecoveryCommand::Status, &handle, None, NOW);
    assert!(ok, "{text}");
    assert!(
        text.contains("Recovery is off. 0 recorded edit(s)"),
        "{text}"
    );
    assert!(text.ends_with(" No workspace is bound."), "{text}");
}

#[test]
fn on_pins_the_retention_text_and_flips_the_session_flag() {
    let sandbox = Sandbox::new("recovery-on");
    let handle = handle_for(&sandbox);
    assert!(
        !handle.is_enabled(),
        "recovery is opt-in: off at acquisition"
    );
    let (ok, text) = run_recovery(RecoveryCommand::On, &handle, Some(&sandbox.ws), NOW);
    assert!(ok, "{text}");
    assert_eq!(text, ON_TEXT, "the retention text is said verbatim");
    assert!(handle.is_enabled(), "the session's own flag flipped on");
}

#[test]
fn on_twice_says_already_on() {
    let sandbox = Sandbox::new("recovery-on-twice");
    let handle = handle_for(&sandbox);
    let (ok, _) = run_recovery(RecoveryCommand::On, &handle, Some(&sandbox.ws), NOW);
    assert!(ok);
    let (ok, text) = run_recovery(RecoveryCommand::On, &handle, Some(&sandbox.ws), NOW);
    assert!(ok, "{text}");
    assert_eq!(text, "Recovery is already on.", "{text}");
    assert!(handle.is_enabled(), "the flag stayed on");
}

#[test]
fn on_without_a_workspace_refuses_and_the_flag_stays_off() {
    let sandbox = Sandbox::new("recovery-on-unbound");
    let handle = handle_for(&sandbox);
    let (ok, text) = run_recovery(RecoveryCommand::On, &handle, None, NOW);
    assert!(!ok, "{text}");
    assert_eq!(
        text,
        "No workspace is bound, so there are no file edits to record. Launch inside a \
         git worktree or pass --workspace <dir>.",
        "{text}"
    );
    assert!(!handle.is_enabled(), "the flag stayed off");
}

#[test]
fn off_flips_the_flag_and_keeps_what_was_recorded() {
    let sandbox = Sandbox::new("recovery-off");
    let handle = handle_for(&sandbox);
    let (ok, _) = run_recovery(RecoveryCommand::On, &handle, Some(&sandbox.ws), NOW);
    assert!(ok);
    record_commit(
        &sandbox,
        "notes.txt",
        EditSource::WorkspaceEdit,
        Some(b"v1\n"),
        b"v2\n",
        NOW,
    );
    let (ok, text) = run_recovery(RecoveryCommand::Off, &handle, Some(&sandbox.ws), NOW);
    assert!(ok, "{text}");
    assert_eq!(text, "Recovery is off.", "{text}");
    assert!(!handle.is_enabled(), "the flag flipped off");
    // What was recorded before the off stays available.
    let (ok, text) = run_recovery(RecoveryCommand::List, &handle, Some(&sandbox.ws), NOW);
    assert!(ok, "{text}");
    assert!(
        text.contains("notes.txt"),
        "the recorded edit is listed: {text}"
    );
}
