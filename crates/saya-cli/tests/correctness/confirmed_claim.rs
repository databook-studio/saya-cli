//! A confirmed claim changes the expected result.
//!
//! `contracts remember` stores a confirmed `time-column order_date` claim on
//! the demo fixture; the scripted model's SQL then follows the supplied
//! context. The test asserts the claim reached the provider request (the
//! `database-contracts` knowledge block) and the rows equal the
//! order_date-based truth.

use super::common;

#[test]
fn confirmed_time_column_changes_expected_result() {
    let fixture = common::build_demo("confirmed-claim");
    remember_time_column(&fixture, "main.main.orders", "order_date");
    enable_assisted_memory(&fixture);
    let count_sql = "SELECT COUNT(*) FROM orders WHERE order_date >= '2026-01-01'";
    let args = serde_json::json!({ "sql": count_sql });
    let mut mock = common::spawn_mock(vec![
        common::tool_call_body("call_count", "bounded_sql_query", args),
        common::text_body("count reported"),
    ]);
    let output = common::run_ask(
        &fixture,
        mock.address(),
        "how many orders since 2026-01-01 does the orders table hold",
    );
    mock.join();
    assert_eq!(
        output.status.code(),
        Some(0),
        "ask must answer: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let bodies = mock.bodies();
    assert_eq!(bodies.len(), 2);
    let first = common::request_json(&bodies[0]);
    let user_turn = first
        .pointer("/messages/1/content")
        .and_then(|c| c.as_str())
        .unwrap();
    assert!(
        user_turn.contains("database-contracts")
            && user_turn.contains("[confirmed] default_time_column  order_date"),
        "the confirmed claim reaches the provider request: {user_turn}",
    );
    let seen = common::tool_result_rows(&bodies[1]);
    assert_eq!(seen.len(), 1);
    let truth = common::run_query(&fixture, count_sql);
    assert_eq!(
        seen[0][0], truth[0][0],
        "the rows equal the order_date-based truth",
    );
    let _ = std::fs::remove_dir_all(&fixture.root);
}

/// Stores a confirmed time-column claim via the real contracts CLI.
fn remember_time_column(fixture: &common::DemoFixture, table: &str, column: &str) {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "--non-interactive",
            "--format",
            "json",
            "--connections",
            fixture.connections.to_str().unwrap(),
            "--profile",
            "demo",
            "contracts",
            "remember",
            table,
            "--kind",
            "time-column",
            "--value",
            column,
        ])
        .env("SAYA_CONFIG_HOME", &fixture.config_home)
        .env("SAYA_STATE_DB", &fixture.state_db)
        .env("HOME", &fixture.root)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "remember must store: {}",
        String::from_utf8_lossy(&output.stderr),
    );
}

/// Assisted memory is off by default; recall needs it on for the claim to
/// reach the provider request. The config file lives at
/// `$SAYA_CONFIG_HOME/saya/config.toml`.
fn enable_assisted_memory(fixture: &common::DemoFixture) {
    let dir = fixture.config_home.join("saya");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("config.toml"), "[memory]\nmode = \"assisted\"\n").unwrap();
}
