use crate::common::*;
use saya_cli::{RenderFormat, TerminalEvent, render_event};

#[test]
fn duckdb_commands_have_stable_process_envelopes_and_safety() {
    let root = std::env::temp_dir().join(format!("saya-cli-duckdb-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("data.duckdb");
    let fixture = duckdb::Connection::open(&database).unwrap();
    fixture
        .execute_batch("CREATE TABLE events (id INTEGER, label VARCHAR); INSERT INTO events VALUES (1, 'one'), (2, 'two');")
        .unwrap();
    drop(fixture);
    let connections = root.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'duckdb'\npath = '{}'\nread_only = true\n",
            database.display()
        ),
    )
    .unwrap();
    let config = root.join("config.toml");
    let state = root.join("state.sqlite3");
    std::fs::write(&config, "[run]\nmax_rows = 1\n").unwrap();
    let base = [
        "--non-interactive",
        "--format",
        "json",
        "--config",
        config.to_str().unwrap(),
        "--connections",
        connections.to_str().unwrap(),
    ];
    let test = run_cli(&base, &["connection", "test", "local"], &state);
    assert_eq!(test.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&test.stdout).contains("\"event\":\"result\""));
    let schema = run_cli(&base, &["connection", "schema", "local"], &state);
    assert_eq!(schema.status.code(), Some(0));
    let schema_output = String::from_utf8_lossy(&schema.stdout);
    assert!(schema_output.contains("\"event\":\"schema\""));
    assert!(schema_output.contains("events"));
    let query = run_cli(
        &[
            "--non-interactive",
            "--format",
            "json",
            "--config",
            config.to_str().unwrap(),
            "--connections",
            connections.to_str().unwrap(),
            "--profile",
            "local",
        ],
        &["query", "--sql", "SELECT id, label FROM events ORDER BY id"],
        &state,
    );
    assert_eq!(query.status.code(), Some(0));
    let query_output = String::from_utf8_lossy(&query.stdout);
    assert!(query_output.contains("\"event\":\"query_result\""));
    assert!(query_output.contains("\"truncated\":true"));
    assert!(query.stderr.is_empty());
    let denied = run_cli(
        &[
            "--non-interactive",
            "--format",
            "json",
            "--config",
            config.to_str().unwrap(),
            "--connections",
            connections.to_str().unwrap(),
            "--profile",
            "local",
        ],
        &["query", "--sql", "DROP TABLE events"],
        &state,
    );
    assert_eq!(denied.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&denied.stderr).contains("\"event\":\"error\""));
    let missing_read_only = root.join("missing-read-only.toml");
    std::fs::write(
        &missing_read_only,
        format!(
            "[profiles.local]\ntype = 'duckdb'\npath = '{}'\n",
            database.display()
        ),
    )
    .unwrap();
    let missing = run_cli(
        &[
            "--non-interactive",
            "--format",
            "json",
            "--connections",
            missing_read_only.to_str().unwrap(),
        ],
        &["connection", "test", "local"],
        &state,
    );
    assert_eq!(missing.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("read_only explicitly"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn query_results_render_as_text_and_json_without_diagnostics() {
    let result = saya_types::QueryResult {
        columns: vec!["id".into()],
        rows: vec![serde_json::json!([1])],
        row_count: 1,
        truncated: true,
        executed_sql: "SELECT id".into(),
    };
    let text = render_event(
        &TerminalEvent::QueryResult {
            result: result.clone(),
        },
        RenderFormat::Text,
    );
    assert_eq!(text.stdout, "id\n1\n[truncated]\n");
    assert!(text.stderr.is_empty());
    let json = render_event(&TerminalEvent::QueryResult { result }, RenderFormat::Json);
    assert!(json.stdout.contains("\"event\":\"query_result\""));
    assert!(json.stderr.is_empty());
}

#[test]
fn duckdb_schema_cache_fallback_refresh_and_interactive_schema_are_stable() {
    use saya_store::{AuditOperation, AuditStore, SqliteStateStore};
    use std::io::Write;
    let root = std::env::temp_dir().join(format!("saya-cli-state-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("state.duckdb");
    duckdb::Connection::open(&database)
        .unwrap()
        .execute_batch("CREATE TABLE cache_events (id INTEGER);")
        .unwrap();
    let connections = root.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.analytics]\ntype = 'duckdb'\npath = '{}'\nread_only = true\n",
            database.display()
        ),
    )
    .unwrap();
    let state = root.join("private-state.sqlite3");
    let globals = [
        "--non-interactive",
        "--format",
        "json",
        "--connections",
        connections.to_str().unwrap(),
    ];
    let first = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args(globals)
        .args(["connection", "schema", "analytics"])
        .env("SAYA_STATE_DB", &state)
        .output()
        .unwrap();
    assert_eq!(first.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&first.stdout).contains("cache_events"));
    let query = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args(globals)
        .args([
            "query",
            "--profile",
            "analytics",
            "--sql",
            "SELECT 'row-secret' AS raw_sql_secret",
        ])
        .env("SAYA_STATE_DB", &state)
        .output()
        .unwrap();
    assert_eq!(query.status.code(), Some(0));
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "--format",
            "ndjson",
            "--connections",
            connections.to_str().unwrap(),
            "--profile",
            "analytics",
        ])
        .env("SAYA_STATE_DB", &state)
        .env("SAYA_SESSION_DIR", root.join("sessions"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"/schema\n/schema refresh\n/exit\n")
        .unwrap();
    let interactive = child.wait_with_output().unwrap();
    assert_eq!(interactive.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&interactive.stdout)
            .matches("\"event\":\"schema\"")
            .count(),
        2
    );
    assert!(interactive.stderr.is_empty());
    std::fs::remove_file(&database).unwrap();
    let cached = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args(globals)
        .args(["connection", "schema", "analytics"])
        .env("SAYA_STATE_DB", &state)
        .output()
        .unwrap();
    assert_eq!(cached.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&cached.stdout).contains("\"event\":\"schema\""));
    assert_eq!(
        String::from_utf8_lossy(&cached.stderr),
        "{\"event\":\"diagnostic\",\"message\":\"Using cached schema metadata; it may be stale.\"}\n"
    );
    let refresh = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args(globals)
        .args(["connection", "schema", "analytics", "--refresh"])
        .env("SAYA_STATE_DB", &state)
        .output()
        .unwrap();
    assert_eq!(refresh.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&refresh.stderr).contains("\"event\":\"error\""));
    let mut failed_repl = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "--format",
            "ndjson",
            "--connections",
            connections.to_str().unwrap(),
            "--profile",
            "analytics",
        ])
        .env("SAYA_STATE_DB", &state)
        .env("SAYA_SESSION_DIR", root.join("failed-sessions"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    failed_repl
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"/schema refresh\n/exit\n")
        .unwrap();
    let failed_repl = failed_repl.wait_with_output().unwrap();
    assert_eq!(failed_repl.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&failed_repl.stderr).contains("\"event\":\"error\""));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let store = SqliteStateStore::new(&state);
    let audits = runtime.block_on(store.recent_audit(100)).unwrap();
    runtime.block_on(store.close());
    drop(store);
    drop(runtime);
    assert!(
        audits
            .iter()
            .all(|row| row.event.profile_id.starts_with("p-") && row.event.profile_id.len() == 66)
    );
    assert!(
        audits
            .iter()
            .any(|row| row.event.operation == AuditOperation::Query)
    );
    assert!(
        audits
            .iter()
            .filter(|row| row.event.operation == AuditOperation::SchemaRefresh)
            .count()
            >= 4
    );
    let decoded_audit = format!("{audits:?}");
    for sentinel in ["analytics", "state.duckdb", "raw_sql_secret", "row-secret"] {
        assert!(!decoded_audit.contains(sentinel), "audit leaked {sentinel}");
    }
    let mut state_bytes = Vec::new();
    for entry in std::fs::read_dir(&root).unwrap().flatten() {
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with("private-state.sqlite3")
        {
            // Tolerate a sidecar (-wal/-shm) checkpointed away between read_dir and
            // read: its bytes are folded into the main state file, which is also read.
            if let Ok(bytes) = std::fs::read(entry.path()) {
                state_bytes.extend(bytes);
            }
        }
    }
    let disk = String::from_utf8_lossy(&state_bytes);
    for sentinel in [
        "analytics",
        "state.duckdb",
        "SELECT 'row-secret'",
        "raw_sql_secret",
        "row-secret",
    ] {
        assert!(!disk.contains(sentinel), "state leaked {sentinel}");
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn schema_command_emits_one_persistence_warning_when_all_state_steps_fail() {
    let root = std::env::temp_dir().join(format!("saya-cli-state-warning-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("data.duckdb");
    duckdb::Connection::open(&database)
        .unwrap()
        .execute_batch("CREATE TABLE events (id INTEGER);")
        .unwrap();
    let connections = root.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.analytics]\ntype = 'duckdb'\npath = '{}'\nread_only = true\n",
            database.display()
        ),
    )
    .unwrap();
    let bad_state = root.join("not-a-database");
    std::fs::create_dir(&bad_state).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "--non-interactive",
            "--format",
            "json",
            "--connections",
            connections.to_str().unwrap(),
            "connection",
            "schema",
            "analytics",
            "--refresh",
        ])
        .env("SAYA_STATE_DB", &bad_state)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"event\":\"schema\""));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr.matches("Local state store unavailable").count(),
        1,
        "{stderr}"
    );
    let _ = std::fs::remove_dir_all(root);
}
