use std::{fs, process::Command as ProcessCommand};

#[test]
fn ask_calls_configured_openai_compatible_provider() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 8192];
        let _ = stream.read(&mut request);
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"answer from mock\"}}]}\n\ndata: [DONE]\n\n";
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
    });
    let root = std::env::temp_dir().join(format!("saya-cli-ask-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let provider_state = root.join("provider-only-state.sqlite3");
    let database = root.join("ask.duckdb");
    duckdb::Connection::open(&database)
        .unwrap()
        .execute_batch("CREATE TABLE revenue (amount INTEGER); INSERT INTO revenue VALUES (7);")
        .unwrap();
    let connections = root.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'duckdb'\npath = '{}'\nread_only = true\n",
            database.display()
        ),
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "--non-interactive",
            "--format",
            "json",
            "--connections",
            connections.to_str().unwrap(),
            "--profile",
            "local",
            "ask",
            "show revenue",
        ])
        .env("SAYA_CONFIG_HOME", &root)
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"))
        .env("SAYA_API_KEY", "mock-secret")
        .env("SAYA_STATE_DB", &provider_state)
        .output()
        .unwrap();
    handle.join().unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("answer from mock"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("mock-secret"));
    assert!(output.stderr.is_empty());
    assert!(
        !provider_state.exists(),
        "provider-only ask must not create an agent-query audit"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// Proves multi-database navigation: with `--profile primary --include-profile warehouse`
/// the agent connects both DuckDB databases and can target each one via the tool
/// `connection` argument. The scripted provider issues two `schema_discovery` calls — one
/// defaulting to the primary and one explicitly targeting `warehouse` — and the audit log
/// must then record two schema inspections against two distinct connection identities.
#[test]
fn ask_navigates_between_included_database_connections() {
    use saya_store::{AuditOperation, AuditStore, SqliteStateStore};
    use std::{
        collections::HashSet,
        io::{Read, Write},
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
            let mut request = [0_u8; 8192];
            let _ = stream.read(&mut request);
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
    let primary_db = root.join("primary.duckdb");
    let warehouse_db = root.join("warehouse.duckdb");
    duckdb::Connection::open(&primary_db)
        .unwrap()
        .execute_batch("CREATE TABLE orders (id INTEGER);")
        .unwrap();
    duckdb::Connection::open(&warehouse_db)
        .unwrap()
        .execute_batch("CREATE TABLE inventory (sku INTEGER);")
        .unwrap();
    let connections = root.join("connections.toml");
    fs::write(
        &connections,
        format!(
            "[profiles.primary]\ntype = 'duckdb'\npath = '{}'\nread_only = true\n\n\
             [profiles.warehouse]\ntype = 'duckdb'\npath = '{}'\nread_only = true\n",
            primary_db.display(),
            warehouse_db.display()
        ),
    )
    .unwrap();
    let state_db = root.join("state.sqlite3");

    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_saya"))
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
