/// The structural denial outranks the grant: a `workspace_write` naming
/// `.git/config` is refused with the harness's typed hygiene error, the
/// seeded bytes survive byte-identical, and the session continues — a held
/// `workspace-write` grant never widens the containment.
#[test]
fn a_headless_ask_run_under_the_grant_keeps_the_structural_deny() {
    let env = env("grant-deny");
    let guarded = env.project.join(".git/config");
    std::fs::write(&guarded, "seed\n").unwrap();
    let scripted = script(vec![
        Box::new(|_| Round::Tool {
            name: "workspace_write",
            arguments: serde_json::json!({"path": ".git/config", "content": "overwritten"}),
        }),
        Box::new(|_| Round::Text("denied")),
    ]);
    let output = run_allowing(
        &env,
        &scripted,
        "ask",
        "overwrite .git/config\n",
        &["workspace-write"],
    );
    let out = stdout(&output);
    assert_eq!(
        output.status.code(),
        Some(0),
        "the denied call is not a failed turn; stdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    assert_eq!(
        std::fs::read_to_string(&guarded).unwrap(),
        "seed\n",
        "the structural refusal writes nothing; stdout:\n{out}"
    );
    let bodies = scripted.bodies.lock().unwrap();
    assert_eq!(bodies.len(), 2, "write, final text");
    let refusal = last_tool_result(&bodies[1]);
    assert!(
        refusal.contains("workspace path names denied hygiene content"),
        "the typed structural refusal reaches the model: {refusal}"
    );
    assert!(
        !refusal.contains("overwritten"),
        "the refusal never carries the call's content: {refusal}"
    );
    drop(bodies);
    assert_tool_flow(
        &out,
        &[
            ("tool_requested", "workspace_write"),
            ("tool_completed", "workspace_write"),
        ],
    );
    assert!(
        out.contains("\"text\":\"denied\""),
        "the session continues after the refusal; stdout:\n{out}"
    );
    let lines = json_lines(&out);
    assert_eq!(
        lines.last().copied().unwrap_or_default(),
        "{\"event\":\"turn_result\",\"status\":\"completed\",\"exit_code\":0,\"tools_completed\":1,\"tools_unfinished\":[]}",
        "the run ends truthfully; stdout:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}
