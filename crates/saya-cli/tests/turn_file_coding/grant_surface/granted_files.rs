/// A headless ask with a launch `workspace-write` grant: the guarded file
/// family is advertised (beside the unconditional read), the digest-chained
/// edit lands, the digest-free create lands, the run completes without any
/// prompt (stdin is null), and the stream ends with the truthful terminal
/// result — three tools completed, nothing unfinished.
#[test]
fn a_headless_ask_run_with_a_workspace_write_grant_creates_and_edits_guarded_files() {
    let env = env("grant-edit");
    let file = env.project.join("src/lib.rs");
    std::fs::write(&file, FIXTURE).unwrap();
    let scripted = script(vec![
        Box::new(|_| Round::Tool {
            name: "workspace_read",
            arguments: serde_json::json!({"path": "src/lib.rs"}),
        }),
        Box::new(move |request| Round::Tool {
            name: "workspace_edit",
            arguments: serde_json::json!({
                "path": "src/lib.rs",
                "old_text": "a - b",
                "new_text": "a + b",
                "expected_digest": digest_from(request),
            }),
        }),
        Box::new(|_| Round::Tool {
            name: "workspace_write",
            arguments: serde_json::json!({"path": "notes/created.md", "content": "hello"}),
        }),
        Box::new(|_| Round::Text("done")),
    ]);
    let output = run_allowing(
        &env,
        &scripted,
        "ask",
        "fix the add function in src/lib.rs and note it in notes/created.md\n",
        &["workspace-write"],
    );
    let out = stdout(&output);
    assert_eq!(
        output.status.code(),
        Some(0),
        "the granted headless run completes with no prompt surface; stdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        EDITED,
        "the guarded edit landed; stdout:\n{out}"
    );
    assert_eq!(
        std::fs::read_to_string(env.project.join("notes/created.md")).unwrap(),
        "hello",
        "the create landed; stdout:\n{out}"
    );
    let bodies = scripted.bodies.lock().unwrap();
    assert_eq!(bodies.len(), 4, "read, edit, create, final text");
    let request = request_json(&bodies[0]);
    let names = tool_names(&request);
    for name in ["workspace_read", "workspace_write", "workspace_edit"] {
        assert!(
            names.contains(&name),
            "the granted family is advertised: {names:?}"
        );
    }
    for name in UNRELATED {
        assert!(
            !names.contains(&name),
            "{name} stays hidden behind one file-family grant: {names:?}"
        );
    }
    let edit_result = last_tool_result(&bodies[2]);
    assert!(
        edit_result.contains("bytes_written"),
        "the edit's result reaches the model: {edit_result}"
    );
    let create_result = last_tool_result(&bodies[3]);
    assert!(
        create_result.contains("bytes_written"),
        "the create's result reaches the model: {create_result}"
    );
    drop(bodies);
    assert_tool_flow(
        &out,
        &[
            ("tool_requested", "workspace_read"),
            ("tool_completed", "workspace_read"),
            ("tool_requested", "workspace_edit"),
            ("tool_completed", "workspace_edit"),
            ("tool_requested", "workspace_write"),
            ("tool_completed", "workspace_write"),
        ],
    );
    assert!(
        out.contains("\"text\":\"done\""),
        "the final text is said; stdout:\n{out}"
    );
    let lines = json_lines(&out);
    assert_eq!(
        lines.last().copied().unwrap_or_default(),
        "{\"event\":\"turn_result\",\"status\":\"completed\",\"exit_code\":0,\"tools_completed\":3,\"tools_unfinished\":[]}",
        "the run ends truthfully; stdout:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}
