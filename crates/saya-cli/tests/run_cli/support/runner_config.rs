/// Writes the user-layer config the binary reads:
/// `<SAYA_CONFIG_HOME>/saya/config.toml`.
fn user_config(env: &TestEnv, toml: &str) {
    let dir = env.root.join("user-config").join("saya");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("config.toml"), toml).unwrap();
}

/// A run that approved `runner:` while `[jobs.runner] program_dir` is
/// undeclared refuses at start (exit 3): an allowlist without its directory
/// approves programs that cannot run. The refusal names the key.
#[test]
fn a_runner_approved_run_refuses_without_a_program_dir() {
    let env = test_root("runner-no-dir");
    let (address, _ready) = mock(Vec::new());
    let output = saya(
        &env,
        &[
            "--non-interactive",
            "run",
            "--allow",
            "runner:bench",
            "measure the harness",
        ],
        &address,
    );
    assert_eq!(
        output.status.code(),
        Some(3),
        "a runner-approved run without a program dir is an admission refusal; stderr: {}",
        stderr(&output)
    );
    let message = stderr(&output);
    assert!(
        message.contains("program_dir"),
        "the refusal must name the missing key: {message}"
    );
    assert!(
        message.contains("runner"),
        "the refusal must say the run approved a runner scope: {message}"
    );
    let _ = fs::remove_dir_all(&env.root);
}

/// A relative `[jobs.runner] program_dir` is a typed resolve error — the
/// canonical form must not depend on the working directory the config was
/// loaded from — refused at config load, before any run exists.
#[test]
fn a_relative_program_dir_is_a_typed_config_error() {
    let env = test_root("runner-relative");
    user_config(&env, "[jobs.runner]\nprogram_dir = \"programs\"\n");
    let (address, _ready) = mock(Vec::new());
    let output = saya(
        &env,
        &[
            "--non-interactive",
            "run",
            "--allow",
            "runner:bench",
            "measure the harness",
        ],
        &address,
    );
    assert_eq!(
        output.status.code(),
        Some(2),
        "a config resolve error is a usage refusal; stderr: {}",
        stderr(&output)
    );
    let message = stderr(&output);
    assert!(
        message.contains("runner.program_dir") && message.contains("absolute"),
        "the refusal must name the field and the requirement: {message}"
    );
    let _ = fs::remove_dir_all(&env.root);
}

/// The placement guard, end to end: a program directory that contains this
/// run's filesystem roots refuses the run with exit 3 and the escape
/// report's why — with programs inside the run tree, one step's child
/// writes the binary the next step's `run_program` validates green and
/// executes. The guard fires before the probe, so this is platform-honest.
#[test]
fn a_program_dir_containing_the_run_tree_refuses_the_run() {
    let env = test_root("runner-placement");
    user_config(
        &env,
        &format!(
            "[jobs.runner]\nallow = [\"bench\"]\nprogram_dir = {}\n",
            toml_path(&env.root)
        ),
    );
    let (address, _ready) = mock(Vec::new());
    let output = saya(
        &env,
        &[
            "--non-interactive",
            "run",
            "--allow",
            "runner:bench",
            "measure the harness",
        ],
        &address,
    );
    assert_eq!(
        output.status.code(),
        Some(3),
        "a program dir containing the run's roots must refuse the run; stderr: {}",
        stderr(&output)
    );
    let message = stderr(&output);
    #[cfg(not(windows))]
    {
        assert!(
            message.contains("overlaps this run's filesystem root"),
            "the refusal must state the containment: {message}"
        );
        assert!(
            message.contains("child write the binary the next step's run_program validates"),
            "the refusal must record the escape it prevents: {message}"
        );
    }
    #[cfg(windows)]
    assert!(
        message.contains("sandbox root is refused for the profile language"),
        "Windows refuses the unsupported sandbox root before placement analysis: {message}"
    );
    let _ = fs::remove_dir_all(&env.root);
}

/// The admission check, end to end: `--allow runner:bench` against a
/// configured directory the probe proves, with no `bench` staged in it,
/// refuses the run at start (exit 3) naming the program and the directory —
/// the operator's staging gap said out loud, never a tool whose every call
/// refuses mid-flight.
#[cfg(target_os = "macos")]
#[test]
fn an_unstaged_program_refuses_the_run_at_start() {
    let env = test_root("runner-unstaged");
    let programs = env.root.join("staged-programs");
    fs::create_dir_all(&programs).unwrap();
    user_config(
        &env,
        &format!(
            "[jobs.runner]\nallow = [\"bench\"]\nprogram_dir = {}\n",
            toml_path(&programs)
        ),
    );
    let (address, _ready) = mock(vec![Scripted {
        body: plan_body(&["measure the harness"]),
        delay_ms: 0,
    }]);
    let output = saya(
        &env,
        &[
            "--non-interactive",
            "run",
            "--allow",
            "runner:bench",
            "measure the harness",
        ],
        &address,
    );
    assert_eq!(
        output.status.code(),
        Some(3),
        "an unstaged program refuses the run at start; stderr: {}",
        stderr(&output)
    );
    let message = stderr(&output);
    assert!(
        message.contains("bench"),
        "the refusal must name the program: {message}"
    );
    assert!(
        message.contains("not in"),
        "the refusal must say the program is missing from the directory: {message}"
    );
    let _ = fs::remove_dir_all(&env.root);
}

/// The grammar mirror holds on the approval surface: `--allow runner:python3`
/// does not parse — the scope parser refuses it as a typed usage error, the
/// same arm an unknown scope token takes, before any run directory exists —
/// naming `interpreter:python3` as the family that approves it. The admission
/// check keeps the refusal list in force beneath this for anything that
/// reaches it.
#[test]
fn an_interpreter_name_refuses_the_run_at_start() {
    let env = test_root("runner-interpreter");
    let programs = env.root.join("staged-programs");
    fs::create_dir_all(&programs).unwrap();
    user_config(
        &env,
        &format!(
            "[jobs.runner]\nallow = [\"bench\"]\nprogram_dir = {}\n",
            toml_path(&programs)
        ),
    );
    let (address, _ready) = mock(Vec::new());
    let output = saya(
        &env,
        &[
            "--non-interactive",
            "run",
            "--allow",
            "runner:python3",
            "measure the harness",
        ],
        &address,
    );
    assert_eq!(
        output.status.code(),
        Some(2),
        "a refused interpreter name is a usage error at the grammar, not an \
         admission refusal; stderr: {}",
        stderr(&output)
    );
    let message = stderr(&output);
    assert!(
        message.contains("python3") && message.contains("shell or interpreter the runner refuses"),
        "the refusal must name the interpreter and its reason: {message}"
    );
    assert!(
        message.contains("`interpreter:python3`"),
        "the refusal must name the family that approves it: {message}"
    );
    let _ = fs::remove_dir_all(&env.root);
}
