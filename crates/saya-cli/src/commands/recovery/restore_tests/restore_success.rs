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
