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
