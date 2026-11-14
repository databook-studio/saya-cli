/// Proves multi-database navigation: with `--profile primary --include-profile warehouse`
/// the agent connects both SQLite databases and can target each one via the tool
/// `connection` argument. The scripted provider issues two `schema_discovery` calls — one
/// defaulting to the primary and one explicitly targeting `warehouse` — and the audit log
/// must then record two schema inspections against two distinct connection identities.
#[test]
fn ask_navigates_between_included_database_connections() {
    use saya_store::{AuditOperation, AuditStore, SqliteStateStore};
    use std::{
        collections::HashSet,
        io::Write,
        net::TcpListener,
        thread,
    };

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());

    // Response 1: one assistant turn with two schema_discovery tool calls (primary + warehouse).
    let call_primary = serde_json::json!({
        "choices": [{"delta": {"tool_calls": [
            {"index": 0, "id": "call_a", "function": {"name": "schema_discovery", "arguments": "{}"}}
        ]}}]
    });
    let call_warehouse = serde_json::json!({
        "choices": [{"delta": {"tool_calls": [
            {"index": 1, "id": "call_b", "function": {"name": "schema_discovery",
                "arguments": serde_json::json!({"connection": "warehouse"}).to_string()}}
        ]}}]
    });
    let tool_calls_body =
        format!("data: {call_primary}\n\ndata: {call_warehouse}\n\ndata: [DONE]\n\n");
    // Response 2: the final answer (no tool calls -> loop terminates).
    let final_chunk =
        serde_json::json!({"choices": [{"delta": {"content": "navigation complete"}}]});
    let final_body = format!("data: {final_chunk}\n\ndata: [DONE]\n\n");

    let handle = thread::spawn(move || {
        for body in [tool_calls_body, final_body] {
            let (mut stream, _) = listener.accept().unwrap();
            crate::common::drain_request(&mut stream);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
            stream.flush().unwrap();
        }
    });

    let root = std::env::temp_dir().join(format!("saya-cli-navigate-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let primary_db = root.join("primary.sqlite");
    let warehouse_db = root.join("warehouse.sqlite");
    crate::common::sqlite_fixture(&primary_db, &["CREATE TABLE orders (id INTEGER);"]);
    crate::common::sqlite_fixture(&warehouse_db, &["CREATE TABLE inventory (sku INTEGER);"]);
    let connections = root.join("connections.toml");
    fs::write(
        &connections,
        format!(
            "[profiles.primary]\ntype = 'sqlite'\npath = '{}'\nread_only = true\n\n\
             [profiles.warehouse]\ntype = 'sqlite'\npath = '{}'\nread_only = true\n",
            primary_db.display(),
            warehouse_db.display()
        ),
    )
    .unwrap();
    let state_db = root.join("state.sqlite3");

    let output = crate::common::saya_binary()
        .args([
            "--non-interactive",
            "--format",
            "json",
            "--connections",
            connections.to_str().unwrap(),
            "--profile",
            "primary",
            "--include-profile",
            "warehouse",
            "ask",
            "inspect both databases",
        ])
        .env("SAYA_CONFIG_HOME", &root)
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"))
        .env("SAYA_API_KEY", "mock-secret")
        .env("SAYA_STATE_DB", &state_db)
        .output()
        .unwrap();
    handle.join().unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("navigation complete"),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let store = SqliteStateStore::new(&state_db);
    let audits = runtime.block_on(store.recent_audit(100)).unwrap();
    runtime.block_on(store.close());
    drop(store);
    drop(runtime);

    let schema_refreshes = audits
        .iter()
        .filter(|row| row.event.operation == AuditOperation::SchemaRefresh)
        .count();
    assert!(
        schema_refreshes >= 2,
        "expected >= 2 schema inspections, got {schema_refreshes}"
    );
    let distinct: HashSet<&str> = audits
        .iter()
        .map(|row| row.event.profile_id.as_str())
        .collect();
    assert_eq!(
        distinct.len(),
        2,
        "agent must inspect two distinct connections (primary + warehouse)"
    );

    let _ = fs::remove_dir_all(root);
}
