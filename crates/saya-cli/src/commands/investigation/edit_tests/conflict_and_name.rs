#[tokio::test]
async fn edit_surfaces_a_stale_expected_revision_as_a_conflict() {
    let root = temp_root("conflict");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let state = SqliteStateStore::new(root.join("state.sqlite3"));
    let id = saved_id(&repo, &runtime, &state).await;
    // Another editor wins the race between this edit's read and its
    // optimistic update. The store refuses a revision it cannot advance;
    // a document parked at the maximum revision makes that refusal
    // deterministic (`expected.checked_add(1)` overflows) without racing
    // two processes.
    let document = fs::read_to_string(document_path(&root, &id)).unwrap();
    fs::write(
        document_path(&root, &id),
        document.replace("\"revision\": 1", "\"revision\": 4294967295"),
    )
    .unwrap();
    let binding_before = fs::read(binding_path(&root, &id)).unwrap();
    let (code, _, err) = run(
        &repo,
        edit_command(&id, Some("Renamed"), None, None, None),
        &runtime,
        &state,
    )
    .await;
    assert_eq!(code, 2, "err: {err}");
    assert!(
        err.contains("changed underneath this command; retry"),
        "err: {err}"
    );
    // Nothing changed: the document keeps the concurrent editor's revision
    // and name, and the binding is untouched.
    let document = fs::read_to_string(document_path(&root, &id)).unwrap();
    assert!(document.contains("\"revision\": 4294967295"), "{document}");
    assert!(document.contains("Order events"), "{document}");
    assert_eq!(fs::read(binding_path(&root, &id)).unwrap(), binding_before);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn edit_refuses_an_invalid_name_with_save_s_message() {
    let root = temp_root("bad-name");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let state = SqliteStateStore::new(root.join("state.sqlite3"));
    let id = saved_id(&repo, &runtime, &state).await;
    let (code, _, err) = run(
        &repo,
        edit_command(&id, Some("   "), None, None, None),
        &runtime,
        &state,
    )
    .await;
    assert_eq!(code, 2, "an empty name is a usage error: {err}");
    assert!(err.contains("name must be 1-80 characters"), "err: {err}");
    let document = fs::read_to_string(document_path(&root, &id)).unwrap();
    assert!(document.contains("\"revision\": 1"), "nothing written");
    let _ = fs::remove_dir_all(root);
}
