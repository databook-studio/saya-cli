/// A run has no runner or interpreter door: its steps run commands with the
/// `shell` tool, so `--allow runner:<program>` and
/// `--allow interpreter:<program>` are usage errors (2) at the grammar,
/// before any run directory exists, and each refusal states the replacement
/// rule and names the scope the user typed. This is a run-surface fact: a
/// session still carries both families until its own surface drops them.
#[test]
fn the_removed_runner_and_interpreter_doors_refuse_at_the_grammar() {
    let env = test_root("removed-doors");
    let (address, _ready) = mock(Vec::new());
    for token in ["runner:report", "interpreter:python3"] {
        let output = saya(
            &env,
            &[
                "--non-interactive",
                "run",
                "--allow",
                token,
                "measure the harness",
            ],
            &address,
        );
        assert_eq!(
            output.status.code(),
            Some(2),
            "`{token}` is a usage refusal at the grammar; stderr: {}",
            stderr(&output)
        );
        let message = stderr(&output);
        assert!(
            message.contains("runner and interpreter scopes were removed"),
            "the refusal must state the removal: {message}"
        );
        assert!(
            message.contains("plan steps run commands with the shell tool"),
            "the refusal must name what runs commands instead: {message}"
        );
        assert!(
            message.contains(token),
            "the refusal must name the scope: {message}"
        );
        assert!(
            !env.runs.exists()
                || fs::read_dir(&env.runs)
                    .map(|mut entries| entries.next().is_none())
                    .unwrap_or(true),
            "the refusal lands before any run directory exists"
        );
    }
    let _ = fs::remove_dir_all(&env.root);
}

/// A relative `[job] program_dir` is a typed resolve error — the
/// canonical form must not depend on the working directory the config was
/// loaded from — refused at config load, before any run exists. The run
/// states a scope it can carry, so the config error is the only refusal in
/// play.
#[test]
fn a_relative_program_dir_is_a_typed_config_error() {
    let env = test_root("runner-relative");
    user_config(&env, "[job]\nprogram_dir = \"programs\"\n");
    let (address, _ready) = mock(Vec::new());
    let output = saya(
        &env,
        &[
            "--non-interactive",
            "run",
            "--allow",
            "workspace-write",
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
        message.contains("job.program_dir") && message.contains("absolute"),
        "the refusal must name the field and the requirement: {message}"
    );
    let _ = fs::remove_dir_all(&env.root);
}
