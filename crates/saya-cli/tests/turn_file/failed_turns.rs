use super::*;

/// A run whose turn file cannot be read fails before its turn: the process
/// exits 2 (`app::run`'s code for a returned error) and stdout still ends
/// with the one promised `turn_result` — `failed` with `exit_code: 2` — as
/// the only stdout line the run produces. The read failure itself is said on
/// stderr, as today.
#[test]
fn a_missing_turn_file_still_ends_with_one_failed_turn_result() {
    let env = test_root("missing-turn");
    let (address, _bodies) = mock("unused");
    let mut args = base_args(&env);
    args.push("--turn-file".into());
    args.push(env.root.join("no-such-turn.md").to_str().unwrap().into());
    let output = saya(
        &env,
        &address,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .output()
    .unwrap();
    assert_eq!(
        output.status.code(),
        Some(2),
        "a run that fails before its turn exits 2; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let lines: Vec<&str> = stdout.lines().filter(|line| !line.is_empty()).collect();
    assert_eq!(
        lines,
        vec![
            "{\"event\":\"turn_result\",\"status\":\"failed\",\"exit_code\":2,\"tools_completed\":0,\"tools_unfinished\":[]}"
        ],
        "the failed result is the only stdout line: {stdout}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains("cannot read --turn-file"),
        "the read failure is said on stderr: {stderr}"
    );
    assert!(
        stderr.contains("\"event\":\"error\""),
        "the failure rides the error event: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// A turn that fails mid-run through a propagated error still ends stdout
/// with the one `turn_result`, `failed` with the process's exit code 2. The
/// cheapest deterministic reproducible: a resumed record whose persisted
/// approval mode no longer parses — the prose turn's mode parse propagates
/// through `run_single_turn` exactly like a store or IO error would.
#[test]
fn a_turn_that_fails_with_a_propagated_error_still_ends_with_one_failed_turn_result() {
    let env = test_root("err-turn");
    let (address, _bodies) = mock("unused");
    let id = "err-turn";
    let record = format!(
        r#"{{"id":"{id}","version":2,"approval_mode":"not-a-mode","turns":[{{"user":"q","assistant":"ok"}}]}}"#
    );
    std::fs::write(env.sessions.join(format!("{id}.json")), record).unwrap();
    let path = env.root.join("instruction.md");
    std::fs::write(&path, "count the orders\n").unwrap();
    let mut args = resume_args(&env, "ndjson");
    args.push("--resume".into());
    args.push(id.into());
    args.push("--turn-file".into());
    args.push(path.to_str().unwrap().into());
    let output = saya(
        &env,
        &address,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .output()
    .unwrap();
    assert_eq!(
        output.status.code(),
        Some(2),
        "a mid-run propagated failure exits 2; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let lines: Vec<&str> = stdout.lines().filter(|line| !line.is_empty()).collect();
    assert_eq!(
        lines.last().copied(),
        Some(
            "{\"event\":\"turn_result\",\"status\":\"failed\",\"exit_code\":2,\"tools_completed\":0,\"tools_unfinished\":[]}"
        ),
        "the failed result ends stdout: {stdout}"
    );
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.contains("\"event\":\"turn_result\""))
            .count(),
        1,
        "exactly one turn_result: {stdout}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains("invalid approval policy"),
        "the propagated error is said: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// The same failed runs in `--format text`: nothing new prints for the
/// result itself (no `turn_result` line on stdout — the error wording is
/// already said), and the exit codes are the same as the ndjson runs: 2 for
/// a run that fails before or during its turn, 5 for the refused save.
#[test]
fn text_format_says_nothing_new_for_failed_runs_and_keeps_the_exit_codes() {
    let env = test_root("text-failures");
    let (address, _bodies) = mock("unused");
    let text_args = |env: &TestEnv| {
        let mut args = base_args(env);
        args[4] = "text".into();
        args
    };
    // A missing turn file: exit 2, no turn_result on stdout.
    let mut args = text_args(&env);
    args.push("--turn-file".into());
    args.push(env.root.join("no-such-turn.md").to_str().unwrap().into());
    let output = saya(
        &env,
        &address,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .output()
    .unwrap();
    assert_eq!(
        output.status.code(),
        Some(2),
        "a failed-before-turn text run exits 2; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("turn_result"),
        "text mode prints no turn_result line: {stdout}"
    );
    // A mid-run propagated failure (the unparsable approval mode): exit 2,
    // no turn_result on stdout.
    let id = "text-err-turn";
    let record = format!(
        r#"{{"id":"{id}","version":2,"approval_mode":"not-a-mode","turns":[{{"user":"q","assistant":"ok"}}]}}"#
    );
    std::fs::write(env.sessions.join(format!("{id}.json")), record).unwrap();
    let path = env.root.join("instruction.md");
    std::fs::write(&path, "count the orders\n").unwrap();
    let mut args = resume_args(&env, "text");
    args.push("--resume".into());
    args.push(id.into());
    args.push("--turn-file".into());
    args.push(path.to_str().unwrap().into());
    let output = saya(
        &env,
        &address,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .output()
    .unwrap();
    assert_eq!(
        output.status.code(),
        Some(2),
        "a mid-run propagated failure in text exits 2; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("turn_result"),
        "text mode prints no turn_result line: {stdout}"
    );
    // The refused save: exit 5, the friendly sentence on stderr, no
    // turn_result on stdout.
    let id = "text-capacity-limit";
    let filler = "x".repeat(3_950_000);
    let record = format!(r#"{{"id":"{id}","turns":[{{"user":"q","assistant":"{filler}"}}]}}"#);
    std::fs::write(env.sessions.join(format!("{id}.json")), record).unwrap();
    let path = env.root.join("big-turn.md");
    std::fs::write(&path, "x".repeat(500_000)).unwrap();
    let mut args = text_args(&env);
    args.push("--resume".into());
    args.push(id.into());
    args.push("--turn-file".into());
    args.push(path.to_str().unwrap().into());
    let output = saya(
        &env,
        &address,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .output()
    .unwrap();
    assert_eq!(
        output.status.code(),
        Some(5),
        "the refused save exits 5 in text too; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("turn_result"),
        "text mode prints no turn_result line: {stdout}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains("4 MiB limit"),
        "the friendly refusal is said in text: {stderr}"
    );
    let limit_lines = stderr
        .lines()
        .filter(|line| line.contains("4 MiB limit"))
        .count();
    assert_eq!(
        limit_lines, 1,
        "the limit sentence is said exactly once in text: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}
