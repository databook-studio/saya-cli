/// SIGINT while a model request is held open: exactly one `turn_result`
/// (status `cancelled`, exit code 5) ends the stdout stream, the retained
/// partial text stays on stdout, exactly one cancellation diagnostic is said
/// on stderr, stdout stays JSON-only — and NO follow-up tool call is started
/// (the scripted round-3 write is never requested, so `after.txt` never
/// exists, and exactly two requests reach the mock).
#[cfg(unix)]
#[test]
fn sigint_during_a_model_request_cancels_with_one_result_and_no_follow_up_effect() {
    let env = env("sigint");
    std::fs::write(env.project.join("src/lib.rs"), FIXTURE).unwrap();
    let scripted = script(vec![
        Box::new(|_| Round::Tool {
            name: "workspace_read",
            arguments: serde_json::json!({"path": "src/lib.rs"}),
        }),
        Box::new(|_| Round::Hold("partial")),
        Box::new(|_| Round::Tool {
            name: "workspace_write",
            arguments: serde_json::json!({"path": "after.txt", "content": "landed"}),
        }),
    ]);
    let mut child = run_background(&env, &scripted, "bypass", "read the file then keep going\n");
    wait_until_holding(&scripted, Duration::from_secs(30));
    child.wait_for_stdout(b"\"text\":\"partial\"", "the retained partial text");
    sigint(child.id());
    let status = child
        .wait_for_exit(Duration::from_secs(20))
        .expect("the run exits after SIGINT");
    assert_eq!(status.code(), Some(5), "a cancelled run exits 5");
    let out = child.stdout();
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
        "{\"event\":\"turn_result\",\"status\":\"cancelled\",\"exit_code\":5,\"tools_completed\":1,\"tools_unfinished\":[]}",
        "the cancelled result is the last stdout line; stdout:\n{out}"
    );
    assert!(
        out.contains("\"text\":\"partial\""),
        "the retained partial result stays on stdout; stdout:\n{out}"
    );
    let err = child.stderr();
    let diagnostics: Vec<&str> = err
        .lines()
        .filter(|line| line.contains("\"event\":\"diagnostic\""))
        .collect();
    assert_eq!(
        diagnostics.len(),
        1,
        "exactly one diagnostic on stderr: {err}"
    );
    assert!(
        diagnostics[0].contains("\"Request cancelled.\""),
        "the cancellation diagnostic is said: {err}"
    );
    assert!(
        !env.project.join("after.txt").exists(),
        "no follow-up write ran after the cancel"
    );
    {
        let bodies = scripted.bodies.lock().unwrap();
        assert_eq!(
            bodies.len(),
            2,
            "exactly two requests reached the mock: the read and the held round"
        );
    }
    // The record saved for the run does not contain the incomplete turn
    // (existing contract, tests/mvp/session.rs).
    let saved = std::fs::read_dir(&env.sessions)
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .map(|entry| std::fs::read_to_string(entry.path()).unwrap())
        .expect("the run saved its session record");
    assert!(
        !saved.contains("read the file then keep going") && !saved.contains("partial"),
        "the incomplete turn is not in the saved record: {saved}"
    );
    drop(child);
    drop(scripted);
    let _ = std::fs::remove_dir_all(&env.root);
}
