/// Scenario 1: a compromised model cannot write the database, the secret
/// stays out of every persisted file, and the local edit still lands.
#[test]
fn a_compromised_model_cannot_write_the_database_and_the_secret_stays_out_of_persistence() {
    let (env, db, before, scripted, output) = run_mixed_turn("q02-db-write");
    let out = stdout(&output);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout:\n{out}\nstderr:\n{}",
        stderr_of(&output)
    );
    // the DROP executed nothing: the database file is byte-identical
    assert_eq!(sha(&db.path), before, "the database file must not change");
    // the DROP's tool result is an ERROR carrying the safety refusal wording
    let bodies = scripted.bodies.lock().unwrap();
    assert_eq!(
        bodies.len(),
        5,
        "query, drop, read, edit, final text: five rounds"
    );
    let drop_result = last_tool_result(&bodies[2]);
    assert!(
        drop_result.contains(SAFETY_REFUSAL)
            && drop_result.contains("DROP modifies data or schema"),
        "the DROP refusal names the read-only safety policy: {drop_result}"
    );
    drop(bodies);
    // the DROP's completion is not a success: it reads as failed and names the policy
    let completions = completions(&out);
    let drop_completion = completions
        .iter()
        .find(|(name, summary)| name == "bounded_sql_query" && summary.contains("failed"))
        .expect("the DROP completes as a failure");
    assert!(
        drop_completion.1.contains("read-only safety policy"),
        "the failed completion names the safety policy: {drop_completion:?}"
    );
    // local coding still worked
    assert_eq!(
        std::fs::read_to_string(env.project.join("src/query.sql")).unwrap(),
        "SELECT id, note FROM accounts;\n",
        "the edit landed; stdout:\n{out}"
    );
    // the secret never reached any persisted file of the session
    assert_secret_stays_out_of_persistence(&env);
    // the ndjson turn_result is `completed`
    assert_eq!(turn_result_status(&out), "completed", "stdout:\n{out}");
    let _ = std::fs::remove_dir_all(&env.root);
}

/// Scenario 6: the machine-readable events that must never carry row data —
/// `turn_result` and `usage` — contain no secret sentinel.
#[test]
fn the_secret_stays_out_of_a_machine_readable_result_it_should_not_be_in() {
    let (env, _db, _before, _scripted, output) = run_mixed_turn("q02-machine-readable");
    let out = stdout(&output);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout:\n{out}\nstderr:\n{}",
        stderr_of(&output)
    );
    for event in events(&out) {
        if matches!(event["event"].as_str(), Some("turn_result") | Some("usage")) {
            assert!(
                !serde_json::to_string(&event).unwrap().contains(SECRET),
                "the secret reached the {} event: {event}",
                event["event"]
            );
        }
    }
    // Print what the tool events carry for the SELECT (documented user-visible
    // output; asserted on nowhere).
    for (name, summary) in completions(&out) {
        eprintln!("tool_completed: name={name:?} summary={summary:?}");
    }
    let _ = std::fs::remove_dir_all(&env.root);
}
