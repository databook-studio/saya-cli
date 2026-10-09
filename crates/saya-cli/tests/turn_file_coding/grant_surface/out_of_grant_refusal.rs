/// The grant opens one family and no other: a scripted `run_command` and a
/// scripted `render_chart` are both answered with the loop's unknown-tool
/// error — never run, never prompted — and no effect lands on disk. The
/// session continues to its final text either way: a denied call is not a
/// failed turn.
#[test]
fn a_headless_ask_run_with_the_grant_still_refuses_an_effect_outside_it() {
    let env = env("grant-refusal");
    let file = env.project.join("src/lib.rs");
    std::fs::write(&file, FIXTURE).unwrap();
    let scripted = script(vec![
        Box::new(|_| Round::Tool {
            name: "run_command",
            arguments: serde_json::json!({"program": "git", "args": ["status"]}),
        }),
        Box::new(|_| Round::Tool {
            name: "render_chart",
            arguments: serde_json::json!({"sql": "SELECT 1", "chart_type": "bar"}),
        }),
        Box::new(|_| Round::Text("refused both")),
    ]);
    let output = run_allowing(
        &env,
        &scripted,
        "ask",
        "run git status and chart SELECT 1\n",
        &["workspace-write"],
    );
    let out = stdout(&output);
    assert_eq!(
        output.status.code(),
        Some(0),
        "a refused call is not a failed turn; stdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    let bodies = scripted.bodies.lock().unwrap();
    assert_eq!(bodies.len(), 3, "command, chart, final text");
    let command_refusal = last_tool_result(&bodies[1]);
    assert!(
        command_refusal.contains("unknown tool 'run_command'"),
        "the loop answers the unadvertised host call: {command_refusal}"
    );
    let chart_refusal = last_tool_result(&bodies[2]);
    assert!(
        chart_refusal.contains("unknown tool 'render_chart'"),
        "the loop answers the unadvertised chart call: {chart_refusal}"
    );
    drop(bodies);
    assert!(
        !env.project.join("notes/leak.md").exists(),
        "no marker file was created"
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        FIXTURE,
        "the seeded file is untouched; stdout:\n{out}"
    );
    assert_tool_flow(
        &out,
        &[
            ("tool_requested", "run_command"),
            ("tool_completed", "run_command"),
            ("tool_requested", "render_chart"),
            ("tool_completed", "render_chart"),
        ],
    );
    assert!(
        out.contains("\"text\":\"refused both\""),
        "the session continues to its final text; stdout:\n{out}"
    );
    let lines = json_lines(&out);
    assert_eq!(
        lines.last().copied().unwrap_or_default(),
        "{\"event\":\"turn_result\",\"status\":\"completed\",\"exit_code\":0,\"tools_completed\":2,\"tools_unfinished\":[]}",
        "the run ends truthfully; stdout:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}
