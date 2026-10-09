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
