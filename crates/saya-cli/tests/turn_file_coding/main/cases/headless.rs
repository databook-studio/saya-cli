/// A headless run without a prompt surface advertises no write-shaped tool:
/// reads stay advertised, a scripted write call is answered with the loop's
/// "unknown tool" error, and nothing is created — headless coding needs an
/// explicit bypass.
#[test]
fn a_headless_ask_run_advertises_no_write_tool() {
    for mode in ["ask", "read-only"] {
        let env = env("no-write-tool");
        std::fs::write(env.project.join("src/lib.rs"), FIXTURE).unwrap();
        let scripted = script(vec![
            Box::new(|_| Round::Tool {
                name: "workspace_write",
                arguments: serde_json::json!({"path": "notes/leak.md", "content": "nope"}),
            }),
            Box::new(|_| Round::Text("nothing written")),
        ]);
        let output = run(&env, &scripted, mode, "write notes/leak.md with nope\n");
        let out = stdout(&output);
        assert_eq!(
            output.status.code(),
            Some(0),
            "mode {mode}; stdout:\n{out}\nstderr:\n{}",
            stderr(&output)
        );
        let bodies = scripted.bodies.lock().unwrap();
        assert_eq!(bodies.len(), 2, "mode {mode}: write, final text");
        let request = request_json(&bodies[0]);
        let names = tool_names(&request);
        assert!(
            names.contains(&"workspace_read"),
            "mode {mode}: reads stay advertised: {names:?}"
        );
        assert!(
            !names.contains(&"workspace_write"),
            "mode {mode}: no write tool without a prompt surface: {names:?}"
        );
        assert!(
            !names.contains(&"workspace_edit"),
            "mode {mode}: no edit tool without a prompt surface: {names:?}"
        );
        let error = last_tool_result(&bodies[1]);
        assert!(
            error.contains("unknown tool 'workspace_write'"),
            "mode {mode}: the loop answers the unadvertised call: {error}"
        );
        drop(bodies);
        assert!(
            !env.project.join("notes/leak.md").exists(),
            "mode {mode}: nothing was created"
        );
        assert!(
            !std::fs::read_to_string(env.project.join("src/lib.rs"))
                .unwrap()
                .is_empty(),
            "mode {mode}: the seeded file is untouched"
        );
        let _ = std::fs::remove_dir_all(&env.root);
    }
}

#[test]
fn no_database_profile_is_needed() {
    let env = env("no-profile");
    // The whole coding flow runs with no connections file anywhere: none in
    // the config home, none in the project, none on the command line.
    assert!(!env.root.join("connections.toml").exists());
    assert!(!env.project.join("connections.toml").exists());
    let file = env.project.join("src/lib.rs");
    std::fs::write(&file, FIXTURE).unwrap();
    let scripted = script(read_edit_rounds(None));
    let output = run(
        &env,
        &scripted,
        "bypass",
        "fix the add function in src/lib.rs\n",
    );
    let out = stdout(&output);
    assert_eq!(
        output.status.code(),
        Some(0),
        "the session starts and completes with no database profile; stdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        EDITED,
        "the full read-edit-report flow ran with no database; stdout:\n{out}"
    );
    drop(scripted);
    let _ = std::fs::remove_dir_all(&env.root);
}

/// The non-empty stdout lines of a run, all required to parse as JSON (the
/// ndjson surface stays machine-readable even when a turn is cancelled).
fn json_lines(out: &str) -> Vec<&str> {
    let lines: Vec<&str> = out.lines().filter(|line| !line.is_empty()).collect();
    for line in &lines {
        serde_json::from_str::<serde_json::Value>(line)
            .unwrap_or_else(|error| panic!("stdout line is not JSON: {line:?}: {error}"));
    }
    lines
}

/// The one terminal result every scripted run must end with: a completed
/// turn reports status `completed`, exit code 0, and the honest tool tally
/// (two tool calls ran and completed), trailing the `complete` event.
#[test]
fn a_completed_scripted_turn_ends_with_one_turn_result() {
    let env = env("turn-result");
    let file = env.project.join("src/lib.rs");
    std::fs::write(&file, FIXTURE).unwrap();
    let scripted = script(read_edit_rounds(None));
    let output = run(
        &env,
        &scripted,
        "bypass",
        "fix the add function in src/lib.rs\n",
    );
    let out = stdout(&output);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    let lines = json_lines(&out);
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.contains("\"event\":\"turn_result\""))
            .count(),
        1,
        "exactly one terminal result; stdout:\n{out}"
    );
    assert_eq!(
        lines.last().copied().unwrap_or_default(),
        "{\"event\":\"turn_result\",\"status\":\"completed\",\"exit_code\":0,\"tools_completed\":2,\"tools_unfinished\":[]}",
        "the terminal result is the last stdout line; stdout:\n{out}"
    );
    assert_eq!(
        lines[lines.len() - 2],
        "{\"event\":\"complete\"}",
        "the result trails the complete event; stdout:\n{out}"
    );
    drop(scripted);
    let _ = std::fs::remove_dir_all(&env.root);
}
