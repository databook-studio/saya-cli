use super::*;

/// Startup notices stay off the machine-readable stdout stream: every line
/// there must remain valid NDJSON, while the human bypass explanation remains
/// visible on stderr.
#[test]
fn bypass_activation_notice_does_not_corrupt_ndjson_stdout() {
    use std::io::Write as _;

    let env = test_root("bypass-ndjson");
    let (address, _bodies) = mock("unused");
    let mut args = base_args(&env);
    args[1] = "bypass".into();
    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .args(&args)
        .current_dir(&env.root)
        .env("SAYA_CONFIG_HOME", &env.root)
        .env("SAYA_SESSION_DIR", &env.sessions)
        .env("SAYA_STATE_DB", env.root.join("state.sqlite3"))
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"))
        .env("SAYA_API_KEY", "mock-secret")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    child.stdin.as_mut().unwrap().write_all(b"/exit\n").unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0));

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines().filter(|line| !line.is_empty()) {
        serde_json::from_str::<serde_json::Value>(line)
            .unwrap_or_else(|error| panic!("stdout line is not NDJSON: {line:?}: {error}"));
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("bypass on: every tool call runs without asking"));
    let _ = std::fs::remove_dir_all(&env.root);
}
