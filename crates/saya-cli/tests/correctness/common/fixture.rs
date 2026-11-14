use std::{fs, path::PathBuf};

/// An isolated, pid-suffixed scratch root for one scenario.
pub(crate) fn test_root(label: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("saya-correctness-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// The isolated fixture a scenario runs: the real demo database plus the
/// config/state roots `saya ask` and `saya query` read.
pub(crate) struct DemoFixture {
    pub root: PathBuf,
    pub connections: PathBuf,
    pub config_home: PathBuf,
    pub state_db: PathBuf,
}

/// Builds the real demo fixture headlessly via SAYA_DEMO_DIR.
pub(crate) fn build_demo(label: &str) -> DemoFixture {
    let root = test_root(label);
    let demo_dir = root.join("demo");
    let config_home = root.join("user-config");
    let state_db = root.join("state.sqlite3");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args(["demo", "--non-interactive", "--format", "json"])
        .env("SAYA_DEMO_DIR", &demo_dir)
        .env("SAYA_CONFIG_HOME", &config_home)
        .env("SAYA_STATE_DB", &state_db)
        .env("HOME", &root)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "demo must build: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let connections = demo_dir.join("connections.toml");
    assert!(connections.exists());
    DemoFixture {
        root,
        connections,
        config_home,
        state_db,
    }
}

/// Runs the real `saya ask` headlessly with the mock as provider.
pub(crate) fn run_ask(
    fixture: &DemoFixture,
    mock_address: &str,
    prompt: &str,
) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "--non-interactive",
            "--format",
            "ndjson",
            "--connections",
            fixture.connections.to_str().unwrap(),
            "--profile",
            "demo",
            "--approval-mode",
            "read-only",
            "--allow-data-sharing",
            "ask",
            prompt,
        ])
        .env("SAYA_CONFIG_HOME", &fixture.config_home)
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{mock_address}/v1"))
        .env("SAYA_API_KEY", "mock-secret")
        .env("SAYA_STATE_DB", &fixture.state_db)
        .env("SAYA_ALLOW_DATA_SHARING", "true")
        .env("HOME", &fixture.root)
        .output()
        .unwrap()
}

/// Direct `saya query`: the independent truth for a scenario.
pub(crate) fn run_query(fixture: &DemoFixture, sql: &str) -> Vec<Vec<serde_json::Value>> {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "--non-interactive",
            "--format",
            "json",
            "--connections",
            fixture.connections.to_str().unwrap(),
            "--profile",
            "demo",
            "query",
            "--sql",
            sql,
        ])
        .env("SAYA_CONFIG_HOME", &fixture.config_home)
        .env("SAYA_STATE_DB", &fixture.state_db)
        .env("HOME", &fixture.root)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "truth query must succeed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let event = stdout
        .lines()
        .find_map(|line| {
            let value: serde_json::Value = serde_json::from_str(line).ok()?;
            (value.get("event")?.as_str() == Some("query_result")).then_some(value)
        })
        .unwrap_or_else(|| panic!("no query_result in: {stdout}"));
    event
        .pointer("/result/rows")
        .and_then(|rows| rows.as_array())
        .unwrap()
        .iter()
        .map(|row| row.as_array().unwrap().clone())
        .collect()
}
