#[test]
fn diff_change_labels_a_created_file_and_a_missing_file() {
    let sandbox = Sandbox::new("diff-labels");
    let created: &[u8] = b"alpha\nbeta\n";
    let record = record_commit(
        &sandbox,
        "made.txt",
        EditSource::WorkspaceWrite,
        None,
        created,
        1_000,
    );
    fs::write(sandbox.ws_root().join("made.txt"), created).expect("file");
    let outcome = diff_change(
        &sandbox.store,
        SESSION,
        Some(&sandbox.ws),
        record.id.as_str(),
    );
    let DiffOutcome::Diff { text, truncated } = &outcome else {
        panic!("expected a diff, got {outcome:?}");
    };
    assert!(!*truncated);
    assert_eq!(
        text.lines().take(4).last(),
        Some("before is empty (created by the edit)")
    );
    assert!(text.contains("(created by the edit)"), "{text}");
    assert!(text.contains("+alpha") && text.contains("+beta"), "{text}");

    let v1: &[u8] = b"before\n";
    let v2: &[u8] = b"after\n";
    fs::write(sandbox.ws_root().join("notes.txt"), v1).expect("seed");
    let replace = record_commit(
        &sandbox,
        "notes.txt",
        EditSource::WorkspaceEdit,
        Some(v1),
        v2,
        2_000,
    );
    fs::write(sandbox.ws_root().join("notes.txt"), v2).expect("edit");
    fs::remove_file(sandbox.ws_root().join("notes.txt")).expect("delete");
    let outcome = diff_change(
        &sandbox.store,
        SESSION,
        Some(&sandbox.ws),
        replace.id.as_str(),
    );
    let DiffOutcome::Diff { text, .. } = &outcome else {
        panic!("expected a diff, got {outcome:?}");
    };
    assert!(text.contains("(file is missing now)"), "{text}");
    assert!(
        text.lines().any(|line| line == "-before"),
        "the missing file's recorded content shows as removed: {text}"
    );
}

#[test]
fn diff_change_of_a_reverted_file_reports_nothing_to_diff() {
    let sandbox = Sandbox::new("diff-same");
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
    fs::write(sandbox.ws_root().join("notes.txt"), v1).expect("revert");
    assert_eq!(
        diff_change(
            &sandbox.store,
            SESSION,
            Some(&sandbox.ws),
            record.id.as_str()
        ),
        DiffOutcome::NothingToDiff
    );
}

#[test]
fn diff_change_refuses_without_a_workspace_or_an_id() {
    let sandbox = Sandbox::new("diff-refuse");
    let record = record_commit(
        &sandbox,
        "notes.txt",
        EditSource::WorkspaceWrite,
        None,
        b"x",
        1_000,
    );
    assert_eq!(
        diff_change(&sandbox.store, SESSION, None, record.id.as_str()),
        DiffOutcome::Refused(RestoreRefusal::NoWorkspace)
    );
    assert_eq!(
        diff_change(&sandbox.store, SESSION, Some(&sandbox.ws), "junk"),
        DiffOutcome::Refused(RestoreRefusal::NotFound)
    );
    assert_eq!(
        diff_change(
            &sandbox.store,
            SESSION,
            Some(&sandbox.ws),
            &format!("ck-{}", "cd".repeat(16)),
        ),
        DiffOutcome::Refused(RestoreRefusal::NotFound)
    );
}

#[test]
fn diff_change_refuses_a_tampered_before_image() {
    let sandbox = Sandbox::new("diff-tamper");
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
    assert_eq!(
        diff_change(
            &sandbox.store,
            SESSION,
            Some(&sandbox.ws),
            record.id.as_str()
        ),
        DiffOutcome::Refused(RestoreRefusal::BeforeImageCorrupt)
    );
}
