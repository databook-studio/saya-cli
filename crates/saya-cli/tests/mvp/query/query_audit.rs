#[test]
fn query_adapter_audits_each_attempt_and_preserves_phase_exits() {
    use saya_store::{AuditOperation, AuditStatus, AuditStore, SqliteStateStore};

    let root = test_root("query-audit-phases");
    let database = root.join("query.sqlite3");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&database)
            .create_if_missing(true);
        let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
        sqlx::query("CREATE TABLE events (id INTEGER)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO events (id) VALUES (1), (2)")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    });
    let connections = root.join("connections.toml");
    let state = root.join("state.sqlite3");
    std::fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\nread_only = true\n",
            database.display()
        ),
    )
    .unwrap();
    let config = root.join("config.toml");
    std::fs::write(&config, "[run]\nmax_rows = 1\nquery_timeout_seconds = 1\n").unwrap();
    let globals = [
        "--non-interactive",
        "--format",
        "json",
        "--config",
        config.to_str().unwrap(),
        "--connections",
        connections.to_str().unwrap(),
        "--profile",
        "local",
    ];

    let success = run_cli(
        &globals,
        &["query", "--sql", "SELECT id FROM events ORDER BY id"],
        &state,
    );
    assert_eq!(
        success.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&success.stderr)
    );
    let success_output = String::from_utf8_lossy(&success.stdout);
    assert!(success_output.contains("\"truncated\":true"));
    assert!(success_output.contains("[1]"));

    let no_profile_connections = root.join("no-profiles.toml");
    std::fs::write(&no_profile_connections, "").unwrap();
    let no_profile = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "--non-interactive",
            "--format",
            "json",
            "--config",
            config.to_str().unwrap(),
            "--connections",
            no_profile_connections.to_str().unwrap(),
            "query",
            "--sql",
            "SELECT 1",
        ])
        .env_remove("SAYA_PROFILE")
        .env("SAYA_CONFIG_HOME", root.join("user-config"))
        .env("SAYA_STATE_DB", &state)
        .output()
        .unwrap();
    assert_eq!(
        no_profile.status.code(),
        Some(4),
        "{}",
        String::from_utf8_lossy(&no_profile.stderr)
    );
    assert!(
        String::from_utf8_lossy(&no_profile.stderr).contains("query requires a selected profile")
    );

    let missing_database = root.join("missing.sqlite3");
    let build_connections = root.join("build-failure.toml");
    std::fs::write(
        &build_connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\nread_only = true\n",
            missing_database.display()
        ),
    )
    .unwrap();
    let build_failure = run_cli(
        &[
            "--non-interactive",
            "--format",
            "json",
            "--config",
            config.to_str().unwrap(),
            "--connections",
            build_connections.to_str().unwrap(),
            "--profile",
            "local",
        ],
        &["query", "--sql", "SELECT 1"],
        &state,
    );
    assert_eq!(build_failure.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&build_failure.stderr).contains("SQLite connection failed"));

    let denied = run_cli(&globals, &["query", "--sql", "DELETE FROM events"], &state);
    assert_eq!(denied.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&denied.stderr).contains(
        "query rejected by read-only safety policy: DELETE modifies data or schema; only reads are allowed"
    ));

    // The local peer forces the real PostgreSQL connector's handshake to fail.
    // This proves connect-phase classification only, not remote cancellation behavior.
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let accept = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    drop(stream);
                    return true;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if std::time::Instant::now() >= deadline {
                        return false;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => panic!("local PostgreSQL test peer accept failed: {error}"),
            }
        }
    });
    let broken_connections = root.join("broken-connections.toml");
    std::fs::write(
        &broken_connections,
        format!(
            "[profiles.local]\ntype = 'postgresql'\nhost = '127.0.0.1'\nport = {port}\ndatabase = 'unused'\nuser = 'unused'\nsslmode = 'disable'\n"
        ),
    )
    .unwrap();
    let broken = run_cli(
        &[
            "--non-interactive",
            "--format",
            "json",
            "--config",
            config.to_str().unwrap(),
            "--connections",
            broken_connections.to_str().unwrap(),
            "--profile",
            "local",
        ],
        &["query", "--sql", "SELECT 1"],
        &state,
    );
    let accepted = accept.join().unwrap();
    assert!(
        accepted,
        "query CLI did not connect to the local PostgreSQL test peer before the 5-second deadline"
    );
    assert_eq!(broken.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&broken.stderr).contains("\"event\":\"error\""));

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let store = SqliteStateStore::new(&state);
    let audits = runtime.block_on(store.recent_audit(100)).unwrap();
    runtime.block_on(store.close());
    drop(store);
    drop(runtime);
    let query_audits: Vec<_> = audits
        .iter()
        .filter(|row| row.event.operation == AuditOperation::Query)
        .collect();
    assert_eq!(query_audits.len(), 4);
    assert_eq!(
        query_audits
            .iter()
            .filter(|row| row.event.status == AuditStatus::Success)
            .count(),
        1
    );
    assert_eq!(
        query_audits
            .iter()
            .filter(|row| row.event.status == AuditStatus::Failure)
            .count(),
        3
    );
    let success_audit = query_audits
        .iter()
        .find(|row| row.event.status == AuditStatus::Success)
        .unwrap();
    assert_eq!(success_audit.event.row_count, Some(1));
    assert_eq!(success_audit.event.truncated, Some(true));
    assert!(
        query_audits
            .iter()
            .filter(|row| row.event.status == AuditStatus::Failure)
            .all(|row| row.event.row_count.is_none() && row.event.truncated.is_none())
    );
    let _ = std::fs::remove_dir_all(root);
}
