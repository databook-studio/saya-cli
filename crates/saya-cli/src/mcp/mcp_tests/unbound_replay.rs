/// FIX-S6a (f): `investigation_run` through the real tool body on an
/// unbound investigation — the state an imported document arrives in —
/// refuses as a tool error even when the model names an allowlisted
/// profile, and writes no binding: a tool call is never the human step
/// that establishes review authority.
#[tokio::test]
async fn an_unbound_investigation_replay_refuses_and_writes_no_binding() {
    let root = std::env::temp_dir().join(format!("saya-mcp-authority-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let local_db = root.join("data.sqlite3");
    let mut runtime = fixture_runtime(
        &format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n",
            local_db.display()
        ),
        "default_profile = 'local'\n",
    );
    runtime.investigations_root = root.join("investigations");
    std::fs::create_dir_all(&runtime.investigations_root).unwrap();
    seed_events(&local_db, "benign").await;
    let state_db = saya_store::SqliteStateStore::new(root.join("state.sqlite3"));
    let repo = InvestigationRepository::new(runtime.investigations_root.clone());
    let id = saved_id(&runtime, &state_db).await;
    let parsed = InvestigationId::parse(&id).unwrap();
    // Unbound: drop the save-time binding — the imported-document shape the
    // step import deliberately leaves to a human.
    std::fs::remove_file(
        runtime
            .investigations_root
            .join("local")
            .join(format!("{id}.json")),
    )
    .unwrap();
    let context = std::sync::Arc::new(super::context::McpContext {
        runtime: runtime.clone(),
        store: state_db,
        replay_slot: tokio::sync::Mutex::new(()),
    });
    let policy = ServePolicy::resolve(&runtime, &["local".to_owned()], None).unwrap();
    // A real request context, the way the served server builds one.
    let (_client, server_io) = tokio::io::duplex(64 * 1024);
    let (server_read, server_write) = tokio::io::split(server_io);
    let running = rmcp::service::serve_directly(
        SayaServer::new(policy.clone(), context.clone()),
        (server_read, server_write),
        None,
    );
    let request_context = rmcp::service::RequestContext::new(
        rmcp::model::RequestId::Number(1),
        running.peer().clone(),
    );
    let request: rmcp::model::CallToolRequestParams = serde_json::from_value(serde_json::json!({
        "name": "investigation_run",
        "arguments": {"id": id, "profile": "local"},
    }))
    .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let response = super::replay_tools::investigation_run(
        &policy,
        &context,
        &request,
        &request_context,
        deadline,
    )
    .await
    .unwrap();
    let _ = running.cancel().await;
    let rendered = match response {
        rmcp::model::CallToolResponse::Complete(result) => {
            assert_eq!(result.is_error, Some(true), "the refusal is a tool error");
            serde_json::to_string(&result).unwrap()
        }
        other => panic!("a tool refusal is a complete result, not {other:?}"),
    };
    assert!(
        rendered.contains("no local review binding"),
        "the refusal names the missing human review: {rendered}"
    );
    assert!(
        repo.get_binding(&parsed).unwrap().is_none(),
        "a refused replay writes no binding"
    );
    let _ = std::fs::remove_dir_all(root);
}
