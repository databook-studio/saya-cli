/// A proven spawn for the toolset tests: the startup probe decides per
/// host, and only the proven arm of `prepare` constructs a `RunnerSpawn`,
/// so this helper — and every test below that uses it — is macOS-only,
/// exactly like the escape battery it mirrors. The program directory sits
/// beside the roots, the staging contract.
fn proven_runner(tag: &str) -> (PathBuf, RunRunner) {
    use std::time::Duration;

    use saya_harness::runner::sandbox::RunSandbox;

    let run_root = std::env::temp_dir().join(format!(
        "saya-run-tools-runner-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&run_root);
    fs::create_dir_all(run_root.join("workspace")).unwrap();
    fs::create_dir_all(run_root.join("state")).unwrap();
    let programs = std::env::temp_dir().join(format!(
        "saya-run-tools-runner-programs-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&programs);
    fs::create_dir_all(&programs).unwrap();
    let sandbox = RunSandbox::new(
        [
            fs::canonicalize(run_root.join("workspace")).unwrap(),
            fs::canonicalize(run_root.join("state")).unwrap(),
        ],
        Vec::<(String, u16)>::new(),
    )
    .expect("the run's roots construct");
    let provision = sandbox.prepare(&programs).expect("preparation must run");
    assert!(
        provision.report().proves_runner(),
        "the probe must prove this host for the wiring to exist:\n{}",
        provision.report().render()
    );
    (
        run_root,
        RunRunner {
            spawn: provision.spawn().expect("proven").clone(),
            timeout: Duration::from_secs(300),
        },
    )
}

/// The inverse pin's second half: the runner tool is in the universe of —
/// and only of — the steps that asked for it. The run-level admission alone
/// (the run approved the scope and the probe proved the host) must not leak
/// `run_program` into a non-asking step, in either its definitions or its
/// executor.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn the_runner_tool_is_in_the_universe_of_the_steps_that_asked_for_it() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace("runner-universe");
    let (_run_root, runner) = proven_runner("universe");

    let mut runner_caps = Capabilities::default();
    runner_caps.runner = Some(RunnerScope::new(vec!["bench".to_owned()]).expect("shaped"));
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            scratch: None,
            fetch: None,
            runner: Some(&runner),
            workspace: &workspace,
            allow_query_data: true,
            cancellation: &cancellation(),
        },
        &[
            step("run the harness", runner_caps.clone()),
            step("read only", Capabilities::default()),
        ],
    );

    // The asking step: run_program appended last, no workspace write tool —
    // the runner alone is not the workspace-write scope.
    assert_eq!(
        names(&built, 0),
        [OPEN_GATE_WITH_CHART, RUNNER_TAIL].concat(),
        "the runner-asking step's universe must end with run_program"
    );
    assert!(
        !names(&built, 0).contains(&"workspace_write".to_string()),
        "the runner alone must not carry the workspace-write tool"
    );
    // The non-asking step: byte-identical to the unchanged universe.
    assert_eq!(
        names(&built, 1),
        OPEN_GATE,
        "a step that did not ask for the runner must never see it"
    );

    // And the executor narrows with the definitions: the non-asking step's
    // composite refuses the name even though the run wired the scope.
    let error = built[1]
        .executor
        .execute("run_program", serde_json::json!({}))
        .await
        .expect_err("the non-asking step has no runner member behind its composite");
    assert_eq!(error, ToolError::UnsupportedTool);
}

/// The slice's headline gate on the approval surface: a step that asked only
/// for one program rejects a different allowlisted one — the tool carries
/// the *step's* narrowed `RunnerScope`, never the run's union, so the
/// enforcement matches the thing the approval view showed.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn a_step_s_narrowed_allowlist_rejects_a_program_it_did_not_ask_for() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace("runner-narrowed");
    let (_run_root, runner) = proven_runner("narrowed");

    // The run approved both programs; this step asked only for `bench`.
    let mut step_caps = Capabilities::default();
    step_caps.runner = Some(RunnerScope::new(vec!["bench".to_owned()]).expect("shaped"));
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            scratch: None,
            fetch: None,
            runner: Some(&runner),
            workspace: &workspace,
            allow_query_data: true,
            cancellation: &cancellation(),
        },
        &[step("measure with the harness", step_caps)],
    );
    assert_eq!(
        names(&built, 0),
        [OPEN_GATE_WITH_CHART, RUNNER_TAIL].concat(),
        "the step's universe must carry run_program"
    );
    let error = built[0]
        .executor
        .execute(
            "run_program",
            serde_json::json!({"program": "ripgrep", "args": []}),
        )
        .await
        .expect_err("a program outside the step's narrowed scope must refuse");
    let ToolError::Runner(message) = &error else {
        panic!("the refusal must be the runner's typed error, got: {error:?}");
    };
    assert!(
        message.contains("ripgrep") && message.contains("allowlist"),
        "the refusal must name the program and the allowlist: {message}"
    );
}

/// The per-step trap (the interpreter approval's design §6): the test a
/// run-wide boolean passes while being wrong. One step of a run holds the
/// interpreter scope, another asks only for the runner; the second step's
/// tool must not contain the interpreter. A run-wide
/// `allow_interpreters` threaded beside the scope machinery makes every step
/// interpreter-capable and cannot pass this — the grant is per-step, like
/// `scratch` and `fetch` before it.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn the_interpreter_grant_does_not_leak_into_the_steps_that_did_not_ask_for_it() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace("interpreter-trap");
    let (_run_root, runner) = proven_runner("interpreter-trap");

    // Step 1 asks for the interpreter; step 2 asks only for a runner
    // program. The run approved the interpreter scope — a run-wide boolean
    // would open the door on both steps.
    let mut interpreter_caps = Capabilities::default();
    interpreter_caps.interpreter =
        Some(InterpreterScope::new(vec!["python3".to_owned()]).expect("refused name"));
    let mut runner_caps = Capabilities::default();
    runner_caps.runner = Some(RunnerScope::new(vec!["bench".to_owned()]).expect("shaped"));
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            scratch: None,
            fetch: None,
            runner: Some(&runner),
            workspace: &workspace,
            allow_query_data: true,
            cancellation: &cancellation(),
        },
        &[
            step("score the fetched benchmark", interpreter_caps),
            step("measure with the harness", runner_caps),
        ],
    );

    // Both steps carry `run_program` — the two families route through one
    // tool, each step's doors narrowed to what that step itself asked for.
    // Neither step carries `render_chart`: the egress union is fetch and
    // runner — the interpreter family deliberately carries none (an
    // interpreter child is wired with an empty `net_allow`).
    assert_eq!(
        names(&built, 0),
        [OPEN_GATE, RUNNER_TAIL].concat(),
        "the interpreter-asking step's universe must carry run_program"
    );
    assert_eq!(
        names(&built, 1),
        [OPEN_GATE_WITH_CHART, RUNNER_TAIL].concat(),
        "the runner-asking step's universe must carry run_program"
    );

    // The asking step's interpreter door is open: a python3 call is not
    // refused by name — it passes the gate and dies on the staged-file
    // battery (nothing is staged here), the refusal an approved interpreter
    // reaches when its bytes are absent. The run-wide boolean passes this
    // half trivially; the next assertion is the one it cannot.
    let error = built[0]
        .executor
        .execute(
            "run_program",
            serde_json::json!({"program": "python3", "args": []}),
        )
        .await
        .expect_err("nothing is staged, so the door's own battery refuses");
    let ToolError::Runner(message) = &error else {
        panic!("the refusal must be the runner's typed error, got: {error:?}");
    };
    assert!(
        message.contains("no allowlisted program file exists"),
        "the asking step's python3 must pass the name gate and reach the file \
         battery, not be refused by name: {message}"
    );

    // The trap: the second step never asked for the interpreter, so its
    // `run_program` must keep the runner's byte-identical name refusal —
    // the run's approval alone grants nothing to a step that did not ask.
    let error = built[1]
        .executor
        .execute(
            "run_program",
            serde_json::json!({"program": "python3", "args": []}),
        )
        .await
        .expect_err("the step that did not ask must refuse the interpreter");
    let ToolError::Runner(message) = &error else {
        panic!("the refusal must be the runner's typed error, got: {error:?}");
    };
    assert!(
        message.contains("shells and interpreters are refused by name"),
        "the non-asking step's refusal must be the runner's byte-identical \
         name refusal: {message}"
    );
}
