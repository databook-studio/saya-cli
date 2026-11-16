struct TestEnv {
    root: PathBuf,
    runs: PathBuf,
    state: PathBuf,
}

/// A per-test scratch root: config home, runs root, and state database all
/// live under one tree (`tests/mvp/common.rs` is the pattern). The workspace
/// is a child of that tree, so the runs root and the state database sit
/// beside it, never inside it: the session binding refuses a workspace that
/// overlaps its own state roots, and on Linux that overlap is real rather
/// than hidden by a symlinked temp path.
fn test_root(label: &str) -> TestEnv {
    let base = std::env::temp_dir().join(format!("saya-run-cli-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let root = base.join("workspace");
    fs::create_dir_all(&root).unwrap();
    TestEnv {
        root,
        runs: base.join("runs"),
        state: base.join("state.sqlite3"),
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        // `root` is the workspace inside the per-test scratch tree; removing
        // its parent takes the runs root and state database with it.
        if let Some(base) = self.root.parent() {
            let _ = fs::remove_dir_all(base);
        }
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
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
