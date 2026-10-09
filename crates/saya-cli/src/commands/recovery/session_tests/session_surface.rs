#[tokio::test]
async fn the_session_surface_records_previews_and_restores_real_edits() {
    let root = temp_dir("recovery-e2e");
    let project = root.join("project");
    fs::create_dir_all(project.join(".git")).expect("the worktree directory");
    let state_dir = root.join("state");
    fs::create_dir_all(&state_dir).expect("the state directory");
    let sessions_root = root.join("sessions");
    let runtime = test_runtime();
    let handle = RecoveryHandle::new(&sessions_root, "e2e-recovery");
    let universe = SessionUniverse::compose_with_launch_and_path(SessionComposition {
        runtime: &runtime,
        explicit: Some(&project),
        pinned_root: None,
        walk_when_unpinned: true,
        cwd: &project,
        state_dir: &state_dir,
        launch: None,
        path: Some("e2e-path".to_owned()),
        scratch: None,
        recovery: Some(handle.clone()),
    })
    .expect("the session composes over the temp worktree");
    let ws = universe.workspace();
    assert!(ws.is_some(), "the explicit root binds a workspace");

    // Flag OFF: the create records nothing and the listing says so.
    let created = write_through(
        &universe,
        serde_json::json!({"path": "notes.txt", "content": "first line\n"}),
    )
    .await;
    assert!(
        created.get("recovery").is_none(),
        "flag off: the create records nothing"
    );
    let (ok, text) = run_recovery(RecoveryCommand::List, &handle, ws.as_deref(), NOW);
    assert!(ok, "{text}");
    assert!(text.contains("No recorded edits"), "{text}");

    // Turning it on says the retention text verbatim.
    let (ok, text) = run_recovery(RecoveryCommand::On, &handle, ws.as_deref(), NOW);
    assert!(ok, "{text}");
    assert_eq!(text, ON_TEXT, "the retention text is said verbatim");
    assert!(handle.is_enabled(), "the session's own flag flipped on");

    // A guarded replace through the executor records exactly one checkpoint.
    let digest = created["digest"]
        .as_str()
        .expect("the write names its digest")
        .to_owned();
    let replaced = write_through(
        &universe,
        serde_json::json!({
            "path": "notes.txt",
            "content": "second line\n",
            "expected_digest": digest,
        }),
    )
    .await;
    assert_eq!(replaced["recovery"], "checkpoint recorded");

    // The listing shows one committed, restorable edit.
    let (ok, text) = run_recovery(RecoveryCommand::List, &handle, ws.as_deref(), NOW);
    assert!(ok, "{text}");
    assert!(
        text.contains("Recorded edits this session (1 of 128"),
        "{text}"
    );
    assert!(text.contains("workspace_write committed"), "{text}");
    assert!(
        text.contains("restorable now: the file still matches the recorded edit"),
        "{text}"
    );
    let full_id = text
        .split_whitespace()
        .find(|token| token.starts_with("ck-"))
        .expect("the listing carries the checkpoint id")
        .to_owned();
    let prefix = format!("ck-{}", &full_id[3..9]);

    // The diff preview shows the exact changed line.
    let (ok, text) = run_recovery(
        RecoveryCommand::Diff(prefix.clone()),
        &handle,
        ws.as_deref(),
        NOW,
    );
    assert!(ok, "{text}");
    assert!(
        text.contains("first line"),
        "the removed line is shown: {text}"
    );
    assert!(
        text.contains("second line"),
        "the added line is shown: {text}"
    );

    // The restore puts the exact before bytes back and appends the reminder.
    let (ok, text) = run_recovery(
        RecoveryCommand::Restore(prefix.clone()),
        &handle,
        ws.as_deref(),
        NOW,
    );
    assert!(ok, "{text}");
    assert!(
        text.contains("Restored notes.txt to its state before the edit"),
        "{text}"
    );
    assert!(
        text.contains("Restoring a file does not undo commands, downloads or database effects."),
        "the reminder line is appended: {text}"
    );
    assert_eq!(
        fs::read(project.join("notes.txt")).expect("the restored file"),
        b"first line\n",
        "the file holds the exact pre-edit bytes"
    );

    // A second restore of the consumed checkpoint: nothing to restore.
    let (ok, text) = run_recovery(
        RecoveryCommand::Restore(prefix),
        &handle,
        ws.as_deref(),
        NOW,
    );
    assert!(!ok, "{text}");
    assert!(
        text.contains("no recorded edit"),
        "the consumed checkpoint is named: {text}"
    );

    // Off: recording stops; an edit made now records nothing.
    let (ok, text) = run_recovery(RecoveryCommand::Off, &handle, ws.as_deref(), NOW);
    assert!(ok, "{text}");
    assert_eq!(text, "Recovery is off.", "{text}");
    assert!(!handle.is_enabled(), "the flag flipped off");
    let replaced = write_through(
        &universe,
        serde_json::json!({
            "path": "notes.txt",
            "content": "third line\n",
            "expected_digest": sha256_hex(b"first line\n"),
        }),
    )
    .await;
    assert!(
        replaced.get("recovery").is_none(),
        "flag off: the edit records nothing"
    );
    let (ok, text) = run_recovery(RecoveryCommand::List, &handle, ws.as_deref(), NOW);
    assert!(ok, "{text}");
    assert!(text.contains("No recorded edits"), "{text}");

    // On again, record one edit, then off: the recorded edit survives and a
    // restore still works with the flag off — restoring is the user's own act.
    let (ok, _) = run_recovery(RecoveryCommand::On, &handle, ws.as_deref(), NOW);
    assert!(ok);
    let replaced = write_through(
        &universe,
        serde_json::json!({
            "path": "notes.txt",
            "content": "fourth line\n",
            "expected_digest": sha256_hex(b"third line\n"),
        }),
    )
    .await;
    assert_eq!(replaced["recovery"], "checkpoint recorded");
    let (ok, text) = run_recovery(RecoveryCommand::Off, &handle, ws.as_deref(), NOW);
    assert!(ok, "{text}");
    let (ok, text) = run_recovery(RecoveryCommand::Status, &handle, ws.as_deref(), NOW);
    assert!(ok, "{text}");
    assert!(
        text.starts_with("Recovery is off. 1 recorded edit(s)"),
        "{text}"
    );
    assert!(
        text.contains(" Recorded edits stay available: /recovery list."),
        "{text}"
    );
    let (ok, list) = run_recovery(RecoveryCommand::List, &handle, ws.as_deref(), NOW);
    assert!(ok, "{list}");
    let full_id = list
        .split_whitespace()
        .find(|token| token.starts_with("ck-"))
        .expect("the listing carries the checkpoint id")
        .to_owned();
    let (ok, text) = run_recovery(
        RecoveryCommand::Restore(full_id),
        &handle,
        ws.as_deref(),
        NOW,
    );
    assert!(ok, "restore works with the flag off: {text}");
    assert_eq!(
        fs::read(project.join("notes.txt")).expect("the restored file"),
        b"third line\n",
        "the file holds the exact pre-edit bytes"
    );

    // A file edited again by hand after the edit was recorded: the restore
    // refuses and the file is untouched.
    let (ok, _) = run_recovery(RecoveryCommand::On, &handle, ws.as_deref(), NOW);
    assert!(ok);
    let replaced = write_through(
        &universe,
        serde_json::json!({
            "path": "notes.txt",
            "content": "sixth line\n",
            "expected_digest": sha256_hex(b"third line\n"),
        }),
    )
    .await;
    assert_eq!(replaced["recovery"], "checkpoint recorded");
    let (ok, list) = run_recovery(RecoveryCommand::List, &handle, ws.as_deref(), NOW);
    assert!(ok, "{list}");
    let full_id = list
        .split_whitespace()
        .find(|token| token.starts_with("ck-"))
        .expect("the listing carries the checkpoint id")
        .to_owned();
    fs::write(project.join("notes.txt"), b"hand edit\n").expect("the hand edit");
    let (ok, text) = run_recovery(
        RecoveryCommand::Restore(full_id),
        &handle,
        ws.as_deref(),
        NOW,
    );
    assert!(!ok, "{text}");
    assert!(
        text.contains("changed after that edit was recorded"),
        "the newer-work refusal is named: {text}"
    );
    assert_eq!(
        fs::read(project.join("notes.txt")).expect("the untouched file"),
        b"hand edit\n",
        "the refused restore left the file untouched"
    );

    let _ = fs::remove_dir_all(&root);
}
