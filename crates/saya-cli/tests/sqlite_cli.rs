use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnectOptions;
use std::process::Command;

#[test]
fn sqlite_commands_have_stable_process_envelopes_and_safety() {
    let root = std::env::temp_dir().join(format!("saya-cli-sqlite-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    let database = root.join("data.sqlite3");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        let options = SqliteConnectOptions::new()
            .filename(&database)
            .create_if_missing(true);
        let pool = SqlitePool::connect_with(options).await.unwrap();
        sqlx::query("CREATE TABLE events (id INTEGER, label TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO events (id, label) VALUES (1, 'one'), (2, 'two')")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    });

    let connections = root.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n",
            database.display()
        ),
    )
    .unwrap();

    let config = root.join("config.toml");
    std::fs::write(&config, "[run]\nmax_rows = 1\n").unwrap();

    let state = root.join("state.sqlite3");

    let base = [
        "--non-interactive",
        "--format",
        "json",
        "--config",
        config.to_str().unwrap(),
        "--connections",
        connections.to_str().unwrap(),
    ];

    // a) connection test local => exit 0, stdout contains "event":"result"
    let test = Command::new(env!("CARGO_BIN_EXE_saya"))
        .args(base)
        .args(["connection", "test", "local"])
        .env("SAYA_STATE_DB", &state)
        .output()
        .unwrap();
    assert_eq!(test.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&test.stdout).contains("\"event\":\"result\""));

    // b) connection schema local => exit 0, stdout contains "event":"schema" AND "events"
    let schema = Command::new(env!("CARGO_BIN_EXE_saya"))
        .args(base)
        .args(["connection", "schema", "local"])
        .env("SAYA_STATE_DB", &state)
        .output()
        .unwrap();
    assert_eq!(schema.status.code(), Some(0));
    let schema_output = String::from_utf8_lossy(&schema.stdout);
    assert!(schema_output.contains("\"event\":\"schema\""));
    assert!(schema_output.contains("events"));

    // c) query --sql "SELECT id, label FROM events ORDER BY id" with --profile local
    //    => exit 0, stdout contains "event":"query_result" AND "truncated":true, stderr empty
    let query = Command::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "--non-interactive",
            "--format",
            "json",
            "--config",
            config.to_str().unwrap(),
            "--connections",
            connections.to_str().unwrap(),
            "--profile",
            "local",
        ])
        .args(["query", "--sql", "SELECT id, label FROM events ORDER BY id"])
        .env("SAYA_STATE_DB", &state)
        .output()
        .unwrap();
    assert_eq!(query.status.code(), Some(0));
    let query_output = String::from_utf8_lossy(&query.stdout);
    assert!(query_output.contains("\"event\":\"query_result\""));
    assert!(query_output.contains("\"truncated\":true"));
    assert!(
        String::from_utf8_lossy(&query.stderr).is_empty(),
        "expected stderr to be empty, got: {}",
        String::from_utf8_lossy(&query.stderr)
    );

    // d) query --sql "DELETE FROM events" with --profile local
    //    => exit code 4 (safety denied), stderr contains "event":"error"
    let denied = Command::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "--non-interactive",
            "--format",
            "json",
            "--config",
            config.to_str().unwrap(),
            "--connections",
            connections.to_str().unwrap(),
            "--profile",
            "local",
        ])
        .args(["query", "--sql", "DELETE FROM events"])
        .env("SAYA_STATE_DB", &state)
        .output()
        .unwrap();
    assert_eq!(denied.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&denied.stderr).contains("\"event\":\"error\""));

    let _ = std::fs::remove_dir_all(&root);
}
