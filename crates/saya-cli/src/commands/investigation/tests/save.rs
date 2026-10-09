#[tokio::test]
async fn save_roundtrip_exact_sql_binding_list_and_show() {
    let root = temp_root("roundtrip");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    // Exact spacing survives: the document keeps the SQL verbatim.
    let sql = "SELECT   id ,  label FROM events";
    let (code, out, err) = run(
        &repo,
        InvestigationCommand::Save {
            name: "Order events".into(),
            description: Some("events in order".into()),
            sql: Some(sql.into()),
            file: None,
            connection: None,
            param_specs: Vec::new(),
        },
        &runtime,
    )
    .await;
    assert_eq!(code, 0, "out: {out} err: {err}");
    assert!(out.contains(
        "Saved exactly as shown. Review the SQL before sharing: literals are stored verbatim."
    ));
    let id = out.lines().next().unwrap().to_string();
    assert!(id.starts_with("order-events-"), "id line: {id}");

    let document = fs::read_to_string(document_path(&root, &id)).unwrap();
    let definition = InvestigationDefinitionV1::from_json_bytes(document.as_bytes()).unwrap();
    assert_eq!(definition.sql, sql, "the SQL must be stored exactly");
    assert_eq!(definition.objects, vec!["events".to_string()]);
    assert_eq!(definition.connection, "local");
    assert_eq!(definition.dialect, SqlDialect::Sqlite);
    assert_eq!(definition.revision, 1);
    assert_eq!(definition.name, "Order events");
    assert!(
        binding_path(&root, &id).exists(),
        "binding beside the document"
    );
    let binding_text = fs::read_to_string(binding_path(&root, &id)).unwrap();
    assert!(binding_text.contains("\"profile_identity\""));

    let (code, out, err) = run(
        &repo,
        InvestigationCommand::List {
            limit: None,
            offset: None,
        },
        &runtime,
    )
    .await;
    assert_eq!(code, 0, "err: {err}");
    assert!(
        out.contains(&format!("{id}  1  sqlite  local  Order events")),
        "list line: {out}"
    );

    let (code, out, err) = run(
        &repo,
        InvestigationCommand::Show { id: id.clone() },
        &runtime,
    )
    .await;
    assert_eq!(code, 0, "err: {err}");
    assert!(out.contains(sql), "show prints the exact SQL: {out}");
    assert!(
        out.contains("local binding: local (reviewed revision 1)"),
        "{out}"
    );
    let identity = expected_identity(&runtime, "local");
    assert!(
        !out.contains(&identity),
        "show must never print the identity: {out}"
    );
    assert!(
        !document.contains(&identity),
        "the document must never carry the identity"
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn save_refuses_write_sql_with_exit_4_and_writes_nothing() {
    let root = temp_root("write-sql");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let (code, out, err) = run(
        &repo,
        InvestigationCommand::Save {
            name: "Wipe".into(),
            description: None,
            sql: Some("DELETE FROM events".into()),
            file: None,
            connection: None,
            param_specs: Vec::new(),
        },
        &runtime,
    )
    .await;
    assert_eq!(code, 4, "out: {out} err: {err}");
    assert!(err.contains("read-only safety policy"), "err: {err}");
    assert!(
        !root.join("investigations").join("local").exists(),
        "nothing written"
    );
    assert!(repo.list(0, 1).unwrap().summaries.is_empty());
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn save_refuses_credential_shaped_sql() {
    let root = temp_root("credential");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let (code, out, err) = run(
        &repo,
        InvestigationCommand::Save {
            name: "Leak".into(),
            description: None,
            sql: Some("SELECT 'password=hunter2'".into()),
            file: None,
            connection: None,
            param_specs: Vec::new(),
        },
        &runtime,
    )
    .await;
    assert_eq!(code, 2, "out: {out} err: {err}");
    assert!(err.contains("credential-shaped text"), "err: {err}");
    assert!(
        repo.list(0, 1).unwrap().summaries.is_empty(),
        "nothing written"
    );
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn save_requires_a_connection_when_none_resolves() {
    let root = temp_root("no-connection");
    // An empty connections file resolves no profile at all.
    let runtime = runtime_at(&root, "");
    let repo = repo_at(&root);
    let (code, out, err) = run(
        &repo,
        InvestigationCommand::Save {
            name: "Lonely".into(),
            description: None,
            sql: Some("SELECT 1".into()),
            file: None,
            connection: None,
            param_specs: Vec::new(),
        },
        &runtime,
    )
    .await;
    assert_eq!(code, 2, "out: {out} err: {err}");
    assert!(
        err.contains("no connection: pass --connection <profile>"),
        "err: {err}"
    );
    assert!(repo.list(0, 1).unwrap().summaries.is_empty());
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn save_refuses_sql_and_file_together() {
    let root = temp_root("both");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    let (code, out, err) = run(
        &repo,
        InvestigationCommand::Save {
            name: "Both".into(),
            description: None,
            sql: Some("SELECT 1".into()),
            file: Some(root.join("unused.sql")),
            connection: None,
            param_specs: Vec::new(),
        },
        &runtime,
    )
    .await;
    assert_eq!(code, 2, "out: {out} err: {err}");
    assert!(err.contains("not both"), "err: {err}");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn save_refuses_invalid_names_and_oversize_sql_with_typed_messages() {
    let root = temp_root("bounds");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    async fn save_with(
        repo: &InvestigationRepository,
        runtime: &RuntimeConfig,
        name: &str,
        sql: String,
    ) -> (i32, String, String) {
        run(
            repo,
            InvestigationCommand::Save {
                name: name.into(),
                description: None,
                sql: Some(sql),
                file: None,
                connection: None,
                param_specs: Vec::new(),
            },
            runtime,
        )
        .await
    }
    let (code, _, err) = save_with(&repo, &runtime, "   ", "SELECT 1".into()).await;
    assert_eq!(code, 2, "an empty name is a usage error");
    assert!(err.contains("name must be 1-80 characters"), "err: {err}");
    let over = format!(
        "SELECT '{}'",
        "x".repeat(saya_types::investigation::MAX_SQL_BYTES)
    );
    let (code, _, err) = save_with(&repo, &runtime, "Big", over).await;
    assert_eq!(code, 2);
    assert!(err.contains("at most 65536 bytes"), "err: {err}");
    let _ = fs::remove_dir_all(root);
}
