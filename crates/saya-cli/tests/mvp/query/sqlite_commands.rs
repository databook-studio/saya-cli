#[test]
fn sqlite_commands_have_stable_process_envelopes_and_safety() {
    let root = std::env::temp_dir().join(format!("saya-cli-sqlite-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("data.sqlite");
    sqlite_fixture(
        &database,
        &[
            "CREATE TABLE events (id INTEGER, label TEXT);",
            "INSERT INTO events VALUES (1, 'one'), (2, 'two');",
        ],
    );
    let connections = root.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\nread_only = true\n",
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
    let _ = std::fs::remove_dir_all(root);
}
