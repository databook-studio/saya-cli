/// A provider that fails every attempt: the turn is failed, said on stderr,
/// and the terminal result names `failed` with exit code 5.
#[test]
fn a_failed_turn_reports_failed_with_exit_five() {
    let env = env("fail-500");
    let scripted = script(vec![
        Box::new(|_| Round::Fail500),
        Box::new(|_| Round::Fail500),
        Box::new(|_| Round::Fail500),
        Box::new(|_| Round::Fail500),
    ]);
    let output = run(&env, &scripted, "bypass", "tell me about the tree\n");
    let out = stdout(&output);
    assert_eq!(
        output.status.code(),
        Some(5),
        "a failed turn exits 5; stdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    let lines = json_lines(&out);
    assert_eq!(
        lines.last().copied().unwrap_or_default(),
        "{\"event\":\"turn_result\",\"status\":\"failed\",\"exit_code\":5,\"tools_completed\":0,\"tools_unfinished\":[]}",
        "the failed result is the last stdout line; stdout:\n{out}"
    );
    assert!(
        stderr(&output).contains("\"event\":\"error\""),
        "the failure is said on stderr: {}",
        stderr(&output)
    );
    drop(scripted);
    let _ = std::fs::remove_dir_all(&env.root);
}
