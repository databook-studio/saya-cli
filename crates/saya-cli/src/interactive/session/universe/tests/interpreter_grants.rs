/// The seam-fix regression (DESIGN §8.1): under `ask`, a granted
/// `interpreter:<program>` token resolves the engine's `Allow` — and the
/// granted interpreter actually reaches execution. The ask offers the token
/// (`grant_token.rs`), the engine honours the grant — but the composition
/// must carry the door too, or the user approved a capability the
/// composition never constructed: a lying approval. The family refusal
/// (an unstaged name) is test 8's pin.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn a_granted_interpreter_actually_runs_under_ask() {
    let project = worktree("interpreter-grant");
    let state = temp_dir("interpreter-grant-state");
    // The trusted config stages the interpreter universe: [jobs.interpreter]
    // allow carries python3, staged beside the runner programs.
    let mut runtime = session_runtime(Some((
        vec!["touch".to_string()],
        Some(PathBuf::from("/usr/bin")),
    )));
    runtime.resolved.jobs.interpreter.allow = vec!["python3".to_string()];
    let universe = compose(&runtime, &project, &state);
    assert!(
        universe.runner.is_some(),
        "the probe proves this host (macOS): the runner composes"
    );

    // The engine half: under ask, the granted token pre-answers the call.
    let definitions = universe.definitions(
        saya_agent::AgentMode::Build,
        ApprovalPolicy::Ask,
        true,
        true,
        false,
        false,
    );
    let run_program = definitions
        .iter()
        .find(|definition| definition.name == "run_program")
        .expect("a proven runner advertises run_program");
    let policy = saya_agent::SessionPolicy::new(ApprovalPolicy::Ask);
    policy.grants().grant("interpreter:python3");
    assert_eq!(
        policy.resolve(&run_program.effect, Some("interpreter:python3")),
        saya_agent::ApprovalDecision::Allow,
        "the granted interpreter token pre-answers the ask"
    );

    // The door half: the granted interpreter call must reach the interpreter
    // door and be admitted — the child's own outcome (whatever it is) is the
    // report, never the family refusal. Today the composition carries no
    // interpreter scope, so the call falls to the runner door and refuses
    // with INTERPRETER_REFUSAL: the user approved a capability the
    // composition never constructed.
    let database = Arc::new(crate::agent::tools::DatabaseTools::new(None, 100, true));
    let executor = universe.executor(Arc::clone(&database), &CancellationToken::new(), false);
    let result = executor
        .execute(
            "run_program",
            serde_json::json!({"program": "python3", "args": ["-c", "print(41 + 1)"]}),
        )
        .await;
    let value = match result {
        Ok(value) => value,
        Err(error) => panic!(
            "the granted interpreter call was refused: {error:?} — the ask approved a \
             capability the composition never constructed"
        ),
    };
    let outcome = serde_json::to_value(&value).unwrap();
    assert!(
        outcome.get("error").is_none(),
        "the call was admitted: the child's own outcome is the report, not a refusal: {outcome}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// The interpreter door's universe is the trusted config's staged
/// `[jobs.interpreter] allow` — built at composition, mode-independently:
/// capability in the composition, consent in the approval engine. With
/// nothing staged, the door does not exist at all (`None`), so an interpreter
/// call keeps the runner door's family refusal.
#[cfg(target_os = "macos")]
#[test]
fn the_session_s_interpreter_door_is_the_staged_config_universe() {
    let project = worktree("interpreter-door");
    let state = temp_dir("interpreter-door-state");
    let mut staged = session_runtime(Some((
        vec!["touch".to_string()],
        Some(PathBuf::from("/usr/bin")),
    )));
    staged.resolved.jobs.interpreter.allow = vec!["python3".to_string(), "perl".to_string()];
    let universe = compose(&staged, &project, &state);
    let runner = universe
        .runner
        .as_ref()
        .expect("the probe proves this host (macOS)");
    let door = runner
        .interpreters
        .as_ref()
        .expect("staged interpreters open the door");
    assert_eq!(door.programs, vec!["python3", "perl"]);

    let unstaged = compose(
        &session_runtime(Some((
            vec!["touch".to_string()],
            Some(PathBuf::from("/usr/bin")),
        ))),
        &project,
        &state,
    );
    let doorless = unstaged
        .runner
        .expect("the runner composes regardless of interpreters");
    assert!(
        doorless.interpreters.is_none(),
        "nothing staged in [jobs.interpreter] allow: the door does not exist"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// An unstaged interpreter keeps the byte-identical family refusal even
/// while the interpreter door is open: the door exists only for names the
/// runner refuses *and* the staged scope carries, so `bash` — refused, not
/// staged — falls back to the runner's own words. (The bytes are pinned on
/// the run surface, `runner/mod.rs`; this pins the session side of the same
/// constant.)
#[cfg(target_os = "macos")]
#[tokio::test]
async fn an_unstaged_interpreter_keeps_the_byte_identical_family_refusal() {
    const FAMILY_REFUSAL: &str = "shells and interpreters are refused by name: \
         an interpreter can spawn arbitrary children with arbitrary argv and would void \
         the typed-argv contract from inside the allowlist";
    let project = worktree("interpreter-family");
    let state = temp_dir("interpreter-family-state");
    let mut runtime = session_runtime(Some((
        vec!["touch".to_string()],
        Some(PathBuf::from("/usr/bin")),
    )));
    runtime.resolved.jobs.interpreter.allow = vec!["python3".to_string()];
    let universe = compose(&runtime, &project, &state);
    assert!(universe.runner.is_some(), "the probe proves this host");
    let database = Arc::new(crate::agent::tools::DatabaseTools::new(None, 100, true));
    let executor = universe.executor(Arc::clone(&database), &CancellationToken::new(), false);
    let result = executor
        .execute(
            "run_program",
            serde_json::json!({"program": "bash", "args": ["-c", "echo hi"]}),
        )
        .await
        .expect_err("bash is not staged: the family refusal stands");
    assert!(
        result.to_string().contains(FAMILY_REFUSAL),
        "the unstaged interpreter keeps the byte-identical family refusal: {result}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}
