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
