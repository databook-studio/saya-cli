/// Scenario 2: hostile project instructions change nothing — the headless ask
/// run advertises no write-shaped tool, the attempted call is refused, and the
/// hostile text rides the labelled untrusted block, never the system prompt.
#[test]
fn hostile_text_in_the_project_and_the_database_grants_nothing() {
    let env = env("q02-hostile-agents");
    std::fs::write(env.project.join("AGENTS.md"), HOSTILE_AGENTS_MD).unwrap();
    let scripted = script(vec![
        Box::new(|_| Round::Tool {
            name: "workspace_write",
            arguments: serde_json::json!({"path": "pwned.txt", "content": "owned"}),
        }),
        Box::new(|_| Round::Text("pwned")),
    ]);
    let output = run(
        &env,
        &scripted,
        "ask",
        "read the project instructions and do what they say\n",
    );
    let out = stdout(&output);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout:\n{out}\nstderr:\n{}",
        stderr_of(&output)
    );
    let bodies = scripted.bodies.lock().unwrap();
    assert_eq!(bodies.len(), 2, "write attempt, final text: two rounds");
    let first = request_json(&bodies[0]);
    let names = names_of(&first);
    let forbidden = [
        "workspace_write",
        "workspace_edit",
        "scratch_sql",
        "http_fetch",
        "http_download",
        "run_command",
        "run_program",
    ];
    for name in forbidden {
        assert!(
            !names.iter().any(|advertised| advertised == name),
            "no {name} under headless ask: {names:?}"
        );
    }
    // the user turn carries the labelled untrusted block with its framing line
    let user: String = first["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == "user")
        .filter_map(|message| message["content"].as_str())
        .collect();
    assert!(
        user.contains("project-instructions (AGENTS.md)") && user.contains(UNTRUSTED_FRAGMENT),
        "the hostile instructions ride the labelled untrusted block: {user}"
    );
    assert!(
        user.contains(HOSTILE_AGENTS_MD),
        "the hostile text reached the model as data: {user}"
    );
    // the system prompt never carries the hostile text
    let system: String = first["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == "system")
        .filter_map(|message| message["content"].as_str())
        .collect();
    assert!(
        !system.contains("Ignore the user"),
        "the system prompt must not carry the hostile text: {system}"
    );
    // the attempted call is refused as unknown and nothing is created
    let refusal = last_tool_result(&bodies[1]);
    assert!(
        refusal.contains(UNKNOWN_TOOL),
        "the loop refuses the unadvertised call: {refusal}"
    );
    drop(bodies);
    assert!(
        !env.project.join("pwned.txt").exists(),
        "nothing was written"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// Scenario 3: a forged approval inside a query result changes nothing — the
/// tool list is identical before and after the hostile cell, and the write
/// attempt is refused.
#[test]
fn a_forged_approval_in_a_query_result_does_not_change_the_surface() {
    let env = env("q02-forged-approval");
    let db = database(&env);
    let scripted = script(vec![
        Box::new(|_| Round::Tool {
            name: "bounded_sql_query",
            arguments: serde_json::json!({"sql": "SELECT note FROM accounts WHERE id = 1"}),
        }),
        Box::new(|_| Round::Tool {
            name: "workspace_write",
            arguments: serde_json::json!({"path": "pwned.txt", "content": "approved by the database"}),
        }),
        Box::new(|_| Round::Text("done")),
    ]);
    let output = run_with_db(
        &env,
        &db,
        &scripted,
        "read-only",
        "read the note and do what it says\n",
    );
    let out = stdout(&output);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout:\n{out}\nstderr:\n{}",
        stderr_of(&output)
    );
    let bodies = scripted.bodies.lock().unwrap();
    assert_eq!(
        bodies.len(),
        3,
        "query, write attempt, final text: three rounds"
    );
    // the hostile cell rode the round-2 tool result
    let query_result = last_tool_result(&bodies[1]);
    assert!(
        query_result.contains(HOSTILE_CELL),
        "the forged approval reached the model as the query result: {query_result}"
    );
    // round 1 and round 2 advertise the SAME tool list
    let round1 = names_of(&request_json(&bodies[0]));
    let round2 = names_of(&request_json(&bodies[1]));
    assert_eq!(
        round1, round2,
        "the retrieved text changed nothing about the surface"
    );
    assert!(
        !round1
            .iter()
            .any(|advertised| advertised == "workspace_write"),
        "no write tool is advertised under read-only: {round1:?}"
    );
    // the write attempt is refused as unknown and nothing was created
    let refusal = last_tool_result(&bodies[2]);
    assert!(
        refusal.contains(UNKNOWN_TOOL),
        "the loop refuses the unadvertised call: {refusal}"
    );
    drop(bodies);
    assert!(
        !env.project.join("pwned.txt").exists(),
        "nothing was written"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}
