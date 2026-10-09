#[tokio::test]
async fn edit_refuses_write_sql_with_exit_4_and_changes_nothing() {
    let root = temp_root("write-sql");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let state = SqliteStateStore::new(root.join("state.sqlite3"));
    let id = saved_id(&repo, &runtime, &state).await;
    let document_before = fs::read(document_path(&root, &id)).unwrap();
    let binding_before = fs::read(binding_path(&root, &id)).unwrap();
    let (code, _, err) = run(
        &repo,
        edit_command(&id, None, None, Some("DELETE FROM events"), None),
        &runtime,
        &state,
    )
    .await;
    assert_eq!(code, 4, "err: {err}");
    assert!(err.contains("read-only safety policy"), "err: {err}");
    assert_eq!(
        fs::read(document_path(&root, &id)).unwrap(),
        document_before,
        "a refused edit changes nothing"
    );
    assert_eq!(fs::read(binding_path(&root, &id)).unwrap(), binding_before);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn edit_refuses_credential_shaped_sql() {
    let root = temp_root("credential");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let state = SqliteStateStore::new(root.join("state.sqlite3"));
    let id = saved_id(&repo, &runtime, &state).await;
    let (code, _, err) = run(
        &repo,
        edit_command(&id, None, None, Some("SELECT 'password=hunter2'"), None),
        &runtime,
        &state,
    )
    .await;
    assert_eq!(code, 2, "err: {err}");
    assert!(err.contains("credential-shaped text"), "err: {err}");
    let document = fs::read_to_string(document_path(&root, &id)).unwrap();
    assert!(document.contains("\"revision\": 1"), "nothing written");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn edit_with_no_fields_is_a_usage_error() {
    let root = temp_root("no-fields");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let state = SqliteStateStore::new(root.join("state.sqlite3"));
    let id = saved_id(&repo, &runtime, &state).await;
    let (code, _, err) = run(
        &repo,
        edit_command(&id, None, None, None, None),
        &runtime,
        &state,
    )
    .await;
    assert_eq!(code, 2, "err: {err}");
    assert!(
        err.contains("edit needs a change") && err.contains("--sql") && err.contains("--name"),
        "err: {err}"
    );
    let document = fs::read_to_string(document_path(&root, &id)).unwrap();
    assert!(document.contains("\"revision\": 1"), "nothing written");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn edit_refuses_sql_and_file_together() {
    let root = temp_root("both");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let state = SqliteStateStore::new(root.join("state.sqlite3"));
    let id = saved_id(&repo, &runtime, &state).await;
    let (code, _, err) = run(
        &repo,
        edit_command(
            &id,
            None,
            None,
            Some("SELECT 1"),
            Some(root.join("unused.sql")),
        ),
        &runtime,
        &state,
    )
    .await;
    assert_eq!(code, 2, "err: {err}");
    assert!(err.contains("not both"), "err: {err}");
    let _ = fs::remove_dir_all(root);
}
