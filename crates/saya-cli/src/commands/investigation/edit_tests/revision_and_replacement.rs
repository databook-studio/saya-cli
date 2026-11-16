#[tokio::test]
async fn edit_sql_bumps_revision_and_the_next_run_refuses_until_revalidate() {
    let root = temp_root("arc");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let state = SqliteStateStore::new(root.join("state.sqlite3"));
    seed_events(&root.join("data.sqlite3")).await;
    let id = saved_id(&repo, &runtime, &state).await;

    let (code, out, err) = run(
        &repo,
        edit_command(
            &id,
            None,
            None,
            Some("SELECT label FROM events WHERE id = 2"),
            None,
        ),
        &runtime,
        &state,
    )
    .await;
    assert_eq!(code, 0, "out: {out} err: {err}");
    assert!(out.contains("\"revision\": 2"), "the new JSON: {out}");
    assert!(
        out.contains("Edited to revision 2. The next run needs --revalidate."),
        "out: {out}"
    );
    assert!(
        out.contains("SELECT label FROM events WHERE id = 2"),
        "the new SQL is echoed: {out}"
    );

    // The document on disk: revision 2, new SQL, recomputed objects, and
    // the immutable id, dialect, connection, and creation time.
    let document = fs::read_to_string(document_path(&root, &id)).unwrap();
    let definition = InvestigationDefinitionV1::from_json_bytes(document.as_bytes()).unwrap();
    assert_eq!(definition.revision, 2);
    assert_eq!(definition.sql, "SELECT label FROM events WHERE id = 2");
    assert_eq!(definition.objects, vec!["events".to_string()]);
    assert_eq!(definition.id.as_str(), id);
    assert_eq!(definition.dialect, SqlDialect::Sqlite);
    assert_eq!(definition.connection, "local");
    assert_eq!(definition.name, "Order events");
    assert_eq!(definition.description.as_deref(), Some("events in order"));

    // The binding is not rewritten: it still reviews revision 1.
    let binding = fs::read_to_string(binding_path(&root, &id)).unwrap();
    assert!(
        binding.contains("\"reviewed_revision\": 1"),
        "binding: {binding}"
    );

    // The next run refuses as stale; --revalidate accepts and replays.
    let (code, _, err) = run(&repo, run_command(&id, false), &runtime, &state).await;
    assert_eq!(code, 2, "err: {err}");
    assert!(err.contains("revision changed"), "err: {err}");
    assert!(err.contains("--revalidate"), "err: {err}");
    let (code, out, err) = run(&repo, run_command(&id, true), &runtime, &state).await;
    assert_eq!(code, 0, "out: {out} err: {err}");
    assert!(out.contains("saved investigation"), "out: {out}");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn edit_keeps_untouched_fields_and_replaces_the_given_ones() {
    let root = temp_root("name-description");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let state = SqliteStateStore::new(root.join("state.sqlite3"));
    let id = saved_id(&repo, &runtime, &state).await;

    // A name edit keeps the description.
    let (code, out, err) = run(
        &repo,
        edit_command(&id, Some("Renamed events"), None, None, None),
        &runtime,
        &state,
    )
    .await;
    assert_eq!(code, 0, "out: {out} err: {err}");
    let document = fs::read_to_string(document_path(&root, &id)).unwrap();
    let definition = InvestigationDefinitionV1::from_json_bytes(document.as_bytes()).unwrap();
    assert_eq!(definition.name, "Renamed events");
    assert_eq!(definition.description.as_deref(), Some("events in order"));
    assert_eq!(definition.sql, "SELECT id, label FROM events ORDER BY id");
    assert_eq!(definition.revision, 2);

    // A description edit keeps the name; the SQL is re-gated even unchanged.
    let (code, out, err) = run(
        &repo,
        edit_command(&id, None, Some("fresh words"), None, None),
        &runtime,
        &state,
    )
    .await;
    assert_eq!(code, 0, "out: {out} err: {err}");
    assert!(
        out.contains("Edited to revision 3. The next run needs --revalidate."),
        "out: {out}"
    );
    let document = fs::read_to_string(document_path(&root, &id)).unwrap();
    let definition = InvestigationDefinitionV1::from_json_bytes(document.as_bytes()).unwrap();
    assert_eq!(definition.name, "Renamed events");
    assert_eq!(definition.description.as_deref(), Some("fresh words"));
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn edit_reads_the_replacement_sql_from_file() {
    let root = temp_root("file");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let state = SqliteStateStore::new(root.join("state.sqlite3"));
    let id = saved_id(&repo, &runtime, &state).await;
    let replacement = root.join("replacement.sql");
    fs::write(&replacement, "SELECT count(*) FROM events").unwrap();
    let (code, _, err) = run(
        &repo,
        edit_command(&id, None, None, None, Some(replacement)),
        &runtime,
        &state,
    )
    .await;
    assert_eq!(code, 0, "err: {err}");
    let document = fs::read_to_string(document_path(&root, &id)).unwrap();
    let definition = InvestigationDefinitionV1::from_json_bytes(document.as_bytes()).unwrap();
    assert_eq!(definition.sql, "SELECT count(*) FROM events");
    let _ = fs::remove_dir_all(root);
}
