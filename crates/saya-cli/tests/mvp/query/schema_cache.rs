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
    #[cfg(windows)]
    assert!(
        String::from_utf8_lossy(&interactive.stderr)
            .contains("Host commands are unavailable on Windows")
    );
    #[cfg(not(windows))]
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
