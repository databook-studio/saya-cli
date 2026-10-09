/// The isolated fixture a scenario runs: the real demo database plus the
/// config/state roots `saya ask` reads. Memory stays off (the default), so
/// recall and the post-turn extraction never spend provider calls.
struct DemoFixture {
    root: PathBuf,
    connections: PathBuf,
    config_home: PathBuf,
    state_db: PathBuf,
}

/// Builds the real demo fixture headlessly via SAYA_DEMO_DIR. Copied from
/// `tests/correctness/common.rs`.
fn build_demo(label: &str) -> DemoFixture {
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
