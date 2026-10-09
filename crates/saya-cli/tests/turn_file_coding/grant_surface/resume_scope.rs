/// A resumed session restarts unstated: run 1 launches with the
/// `workspace-write` grant and the family is advertised; run 2 re-opens the
/// saved record WITHOUT `--allow` and its first request no longer advertises
/// `workspace_write`/`workspace_edit` — the scripted write is answered with
/// the unknown-tool error and nothing lands. Reads stay advertised either
/// way, so the contrast is the grant, not the surface. One server serves
/// both runs: a resumed session dials the endpoint its record persisted
/// (run 1's base URL), so run 2 must reach the same scripted listener.
#[test]
fn a_resumed_session_does_not_restore_the_launch_grant() {
    let env = env("grant-resume");
    std::fs::write(env.project.join("src/lib.rs"), FIXTURE).unwrap();
    let scripted = script(vec![
        // Run 1: a read and the final text.
        Box::new(|_| Round::Tool {
            name: "workspace_read",
            arguments: serde_json::json!({"path": "src/lib.rs"}),
        }),
        Box::new(|_| Round::Text("read done")),
        // Run 2: the resumed session's write attempt and its final text.
        Box::new(|_| Round::Tool {
            name: "workspace_write",
            arguments: serde_json::json!({"path": "notes/leak.md", "content": "nope"}),
        }),
        Box::new(|_| Round::Text("stopped")),
    ]);
    let first = run_allowing(
        &env,
        &scripted,
        "ask",
        "look at src/lib.rs\n",
        &["workspace-write"],
    );
    let first_out = stdout(&first);
    assert_eq!(
        first.status.code(),
        Some(0),
        "run 1 completes; stdout:\n{first_out}\nstderr:\n{}",
        stderr(&first)
    );
    {
        let bodies = scripted.bodies.lock().unwrap();
        assert_eq!(bodies.len(), 2, "run 1: read, final text");
        let request = request_json(&bodies[0]);
        let names = tool_names(&request);
        for name in ["workspace_write", "workspace_edit"] {
            assert!(
                names.contains(&name),
                "run 1 advertised the granted family: {names:?}"
            );
        }
    }
    let id = saved_session_id(&env);
    let second = run_resuming(
        &env,
        &scripted,
        "ask",
        "now write notes/leak.md\n",
        &id,
        &[],
    );
    let out = stdout(&second);
    assert_eq!(
        second.status.code(),
        Some(0),
        "run 2 completes; stdout:\n{out}\nstderr:\n{}",
        stderr(&second)
    );
    {
        let bodies = scripted.bodies.lock().unwrap();
        assert_eq!(bodies.len(), 4, "run 1's two, then run 2's write and text");
        let request = request_json(&bodies[2]);
        let names = tool_names(&request);
        assert!(
            names.contains(&"workspace_read"),
            "reads stay advertised on the resumed run: {names:?}"
        );
        for name in ["workspace_write", "workspace_edit"] {
            assert!(
                !names.contains(&name),
                "run 2 does not advertise the lost grant: {names:?}"
            );
        }
        let error = last_tool_result(&bodies[3]);
        assert!(
            error.contains("unknown tool 'workspace_write'"),
            "the resumed run answers the unadvertised call: {error}"
        );
    }
    assert!(
        !env.project.join("notes/leak.md").exists(),
        "the resumed run wrote nothing"
    );
    assert!(
        out.contains("\"text\":\"stopped\""),
        "run 2 ends with its final text; stdout:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}
