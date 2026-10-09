#[tokio::test]
async fn cached_schema_is_explicit_and_agent_query_audit_omits_sql() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("saya-agent-state-{stamp}"));
    let path = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&path);
    let profile = "quoted profile";
    let key = crate::profile_identity::profile_identity(
        profile,
        &saya_types::DatabaseProfile::DuckDb {
            path: "agent-state.duckdb".into(),
            read_only: Some(true),
        },
        std::path::Path::new("/agent-test/connections.toml"),
    )
    .as_str()
    .to_owned();
    store
        .upsert_schema(&key, &SchemaTree::default())
        .await
        .unwrap();
    let cached = schema(&Failing, Some(&store), Some(&key)).await.unwrap();
    assert_eq!(
        cached["diagnostic"],
        "using cached schema because live refresh failed: schema discovery failed: server sentinel"
    );
    let mut registry = ConnectionRegistry::new("primary");
    registry.insert(
        "primary",
        ConnectionEntry {
            connector: Box::new(Failing),
            dialect: SqlDialect::DuckDb,
            profile_id: Some(key.clone()),
        },
    );
    let operation = registry
        .resolve_for_operation(None)
        .expect("test registry resolves its primary");
    assert!(
        query_with_context(
            &operation,
            "SELECT 'raw SQL sentinel'",
            1,
            Some(&store),
            Some(&key),
            &saya_agent::ToolExecutionContext {
                result_cap: 1024,
                cancellation: saya_agent::CancellationToken::new(),
            },
            None,
        )
        .await
        .is_err()
    );
    let audit = store.recent_audit(10).await.unwrap();
    assert!(
        audit
            .iter()
            .any(|row| row.event.operation == AuditOperation::AgentQuery)
    );
    assert!(!String::from_utf8_lossy(&fs::read(&path).unwrap()).contains("raw SQL sentinel"));
    let _ = fs::remove_dir_all(root);
}
