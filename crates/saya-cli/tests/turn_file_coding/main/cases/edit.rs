#[test]
fn scripted_read_then_edit_changes_the_file_and_exits_zero() {
    let env = env("edit");
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
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        EDITED,
        "the edit landed and nothing else changed; stdout:\n{out}"
    );
    assert_tool_flow(
        &out,
        &[
            ("tool_requested", "workspace_read"),
            ("tool_completed", "workspace_read"),
            ("tool_requested", "workspace_edit"),
            ("tool_completed", "workspace_edit"),
        ],
    );
    assert!(
        out.contains("\"text\":\"fixed\""),
        "the final text is said; stdout:\n{out}"
    );
    let bodies = scripted.bodies.lock().unwrap();
    assert_eq!(
        bodies.len(),
        3,
        "read, edit, final text: exactly three rounds"
    );
    let edit_result = last_tool_result(&bodies[2]);
    assert!(
        edit_result.contains("bytes_written"),
        "the round-3 body carries the edit result: {edit_result}"
    );
    drop(bodies);
    let _ = std::fs::remove_dir_all(&env.root);
}

#[test]
fn a_stale_digest_edit_is_refused_and_leaves_the_file_unchanged() {
    let env = env("stale-digest");
    let file = env.project.join("src/lib.rs");
    std::fs::write(&file, FIXTURE).unwrap();
    let scripted = script(read_edit_rounds(Some("0".repeat(64))));
    let output = run(
        &env,
        &scripted,
        "bypass",
        "fix the add function in src/lib.rs\n",
    );
    let out = stdout(&output);
    // A refused tool call is not a failed turn: the final text still arrives.
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        FIXTURE,
        "the refused edit leaves the file byte-identical; stdout:\n{out}"
    );
    let bodies = scripted.bodies.lock().unwrap();
    assert_eq!(bodies.len(), 3, "the loop still reaches the final text");
    // The refusal reaches the model as the edit call's tool result.
    let refusal = last_tool_result(&bodies[2]);
    assert!(
        refusal.contains("file changed since measured"),
        "the refusal names the moved anchor: {refusal}"
    );
    // The refusal must not hand over the file's content.
    assert!(
        !refusal.contains("a - b"),
        "the refusal never carries file content: {refusal}"
    );
    // The edit refusal states the file's CURRENT digest and size (the
    // `WorkspaceEditMoved` contract) so the model can retry after a re-read —
    // it carries state metadata, never content (asserted above). Unlike
    // `workspace_write`'s refusals, which carry no digest at all.
    let observed = digest_from(&bodies[1]);
    assert!(
        refusal.contains(&observed),
        "the edit refusal names the current digest so a retry is possible: {refusal}"
    );
    drop(bodies);
    let _ = std::fs::remove_dir_all(&env.root);
}

#[test]
fn replacing_an_existing_file_without_a_digest_is_refused() {
    let env = env("replace-no-digest");
    let file = env.project.join("src/lib.rs");
    std::fs::write(&file, FIXTURE).unwrap();
    let scripted = script(vec![
        Box::new(|_| Round::Tool {
            name: "workspace_write",
            arguments: serde_json::json!({"path": "src/lib.rs", "content": "x"}),
        }),
        Box::new(|_| Round::Text("left it alone")),
    ]);
    let output = run(&env, &scripted, "bypass", "write src/lib.rs with just x\n");
    let out = stdout(&output);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        FIXTURE,
        "the refused replace leaves the file byte-identical; stdout:\n{out}"
    );
    assert_tool_flow(
        &out,
        &[
            ("tool_requested", "workspace_write"),
            ("tool_completed", "workspace_write"),
        ],
    );
    let bodies = scripted.bodies.lock().unwrap();
    assert_eq!(bodies.len(), 2, "write, final text: exactly two rounds");
    let refusal = last_tool_result(&bodies[1]);
    assert!(
        refusal.contains("expected_digest"),
        "the refusal names the guard: {refusal}"
    );
    drop(bodies);
    let _ = std::fs::remove_dir_all(&env.root);
}

#[test]
fn creating_a_new_file_needs_no_digest() {
    let env = env("create-new");
    let scripted = script(vec![
        Box::new(|_| Round::Tool {
            name: "workspace_write",
            arguments: serde_json::json!({"path": "notes/new.md", "content": "hello"}),
        }),
        Box::new(|_| Round::Text("wrote it")),
    ]);
    let output = run(
        &env,
        &scripted,
        "bypass",
        "create notes/new.md with hello\n",
    );
    let out = stdout(&output);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    assert_eq!(
        std::fs::read_to_string(env.project.join("notes/new.md")).unwrap(),
        "hello",
        "the file was created with exactly that content; stdout:\n{out}"
    );
    assert_tool_flow(
        &out,
        &[
            ("tool_requested", "workspace_write"),
            ("tool_completed", "workspace_write"),
        ],
    );
    let bodies = scripted.bodies.lock().unwrap();
    assert_eq!(bodies.len(), 2, "write, final text: exactly two rounds");
    let result = last_tool_result(&bodies[1]);
    assert!(
        result.contains("bytes_written"),
        "the create result reaches the model: {result}"
    );
    drop(bodies);
    let _ = std::fs::remove_dir_all(&env.root);
}
