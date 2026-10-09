struct TestEnv {
    root: PathBuf,
    runs: PathBuf,
    state: PathBuf,
}

/// A per-test scratch root: config home, runs root, and state database all
/// live under one tree (`tests/mvp/common.rs` is the pattern).
fn test_root(label: &str) -> TestEnv {
    let root = std::env::temp_dir().join(format!("saya-run-cli-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    TestEnv {
        root: root.clone(),
        runs: root.join("runs"),
        state: root.join("state.sqlite3"),
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Encodes a filesystem path as a TOML string, including Windows separators.
fn toml_path(path: &std::path::Path) -> String {
    toml::Value::String(path.to_string_lossy().into_owned()).to_string()
}

/// Runs the real `saya` binary against the scratch tree and `base_url`.
fn saya(env: &TestEnv, args: &[&str], address: &str) -> Output {
    ProcessCommand::new(env!("CARGO_BIN_EXE_saya"))
        .args(args)
        .current_dir(&env.root)
        .env("SAYA_CONFIG_HOME", env.root.join("user-config"))
        .env("SAYA_RUNS_DIR", &env.runs)
        .env("SAYA_STATE_DB", &env.state)
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"))
        .env("SAYA_API_KEY", "mock-secret")
        .output()
        .unwrap()
}

/// The newest run id from `saya run list`'s first line.
fn newest_run_id(output: &Output) -> String {
    stdout(output)
        .lines()
        .next()
        .unwrap_or_default()
        .split('\t')
        .next()
        .unwrap_or_default()
        .to_string()
}
