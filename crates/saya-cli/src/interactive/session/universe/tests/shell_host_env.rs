/// The canary root the outer run points its inner run at.
const SHELL_ENV_CANARY_ROOT: &str = "SAYA_SSHELL_CANARY_ROOT";
/// The inner run that dumps the session shell's host child environment.
const SHELL_ENV_TEST: &str =
    "interactive::session::universe::tests::a_session_host_run_does_not_inherit_saya_secrets";

/// A host run of the session's shell does not inherit saya's own secrets: a
/// planted `SAYA_` variable — the namespace the host policy removes wholesale —
/// is named for removal and never reaches the child, while the rest of this
/// process's environment does. Driven through a self-re-exec so no `set_var`
/// ever touches the shared test binary's own environment, the way the coding
/// root's own host-env test does it.
#[tokio::test]
async fn a_session_host_run_does_not_inherit_saya_secrets() {
    if let Some(dir) = std::env::var_os(SHELL_ENV_CANARY_ROOT) {
        inner_session_host_env(PathBuf::from(dir)).await;
        return;
    }
    let guard = temp_dir("shell-host-env-outer");
    let status = std::process::Command::new(
        std::env::current_exe().expect("the test binary path resolves"),
    )
    .arg("--exact")
    .arg(SHELL_ENV_TEST)
    .env(SHELL_ENV_CANARY_ROOT, &guard)
    .env("SAYA_X_SECRET", "saya-planted-secret")
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .status()
    .expect("the self-re-exec must spawn");
    assert!(
        status.success(),
        "the inner shell-env test must pass: {status}"
    );
    let text = fs::read_to_string(guard.join("host-env.txt"))
        .expect("the inner run must record the host child's env");
    assert!(
        !text.contains("saya-planted-secret"),
        "the host child must not inherit a saya secret: {text}"
    );
    assert!(
        !text.contains("SAYA_X_SECRET="),
        "the host child must not even carry the name: {text}"
    );
    // The positive control: the child ran, and it inherited the rest of this
    // process's environment. `PATH` is the name the coding root's own
    // host-env test pins for the same reason.
    assert!(
        text.contains("PATH="),
        "the host child inherits the rest of this process's environment: {text}"
    );
    let _ = fs::remove_dir_all(&guard);
}

/// The inner half of [`a_session_host_run_does_not_inherit_saya_secrets`]:
/// composes a session universe over the planted environment, pins that the
/// planted name is the one the host policy removes, and dumps the host
/// child's environment into the canary root.
async fn inner_session_host_env(dir: PathBuf) {
    let project = worktree("shell-host-env-inner");
    let state = temp_dir("shell-host-env-inner-state");
    let runtime = session_runtime();
    // The mechanism, pinned where the capture cannot confound it: the planted
    // name is on the list the host child's environment is built by removing.
    let removed = crate::agent::tools::shell_policy::host_policy(&runtime).remove_env;
    assert!(
        removed.iter().any(|name| name == "SAYA_X_SECRET"),
        "the planted saya variable is named for removal: {removed:?}"
    );
    let universe = compose(&runtime, &project, &state);
    // `bypass` is the mode that runs a host call with nothing to ask: this
    // inner run is about what the host child inherits, not about the asking.
    let Some(shell) = universe.shell(ApprovalPolicy::Bypass, None, CancellationToken::new())
    else {
        fs::write(dir.join("host-env.txt"), b"no shell").expect("record the failure");
        return;
    };
    let outcome = shell
        .run_call(&serde_json::json!({"command": "env"}))
        .await
        .expect("the env dump must run");
    fs::write(
        dir.join("host-env.txt"),
        outcome["stdout"].as_str().unwrap_or_default(),
    )
    .expect("record the child's env");
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}
