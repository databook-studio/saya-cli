use super::*;

/// The turn file's bytes — code fences, blank lines, trailing spaces —
/// reach the turn byte-exact. The piped stdin path cannot carry this: it
/// reads line-by-line and `trim_end`s each line, so trailing spaces die,
/// blank lines are skipped, and one multi-line instruction arrives as many
/// turns. CRLF is damaged too (a folded line never survives `\r\n`).
#[test]
fn a_turn_file_is_read_byte_exact() {
    let env = test_root("byte-exact");
    let (address, bodies) = mock("done");
    let turn = "Count orders per region.\n\n```sql\nSELECT 1;   \n```\nTrailing spaces here:   \n\nDone.\n";
    assert!(turn.contains("   \n"), "fixture must carry trailing spaces");
    assert!(turn.contains("\n\n"), "fixture must carry blank lines");
    let path = env.root.join("instruction.md");
    std::fs::write(&path, turn).unwrap();

    let mut args = base_args(&env);
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
        Some(0),
        "completed turn exits zero; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let seen = bodies.lock().unwrap();
    assert_eq!(seen.len(), 1, "exactly one provider round ran");
    let body = String::from_utf8_lossy(&seen[0]).into_owned();
    // The raw HTTP body must carry the turn's layout verbatim: trailing
    // spaces, blank lines, and the fence survive JSON-string escaping but
    // must not be folded, trimmed, or split.
    for fragment in [
        "Trailing spaces here:   ",
        "```sql\\nSELECT 1;   \\n```",
        "\\n\\n",
    ] {
        assert!(
            body.contains(fragment),
            "provider request must carry {fragment:?}; body: {body}"
        );
    }
    let _ = std::fs::remove_dir_all(&env.root);
}

/// Exactly one turn runs, then the process exits — no waiting on stdin.
/// stdin is `/dev/null` here: if the flag looped back into the piped reader,
/// the process would idle instead of exiting.
#[test]
fn exactly_one_turn_runs_then_the_process_exits() {
    let env = test_root("one-turn");
    let (address, _bodies) = mock("done");
    let path = env.root.join("instruction.md");
    std::fs::write(&path, "/help\n").unwrap();

    let mut args = base_args(&env);
    args.push("--turn-file".into());
    args.push(path.to_str().unwrap().into());
    let started = std::time::Instant::now();
    let output = saya(
        &env,
        &address,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .output()
    .unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(60),
        "the process must exit after one turn, not wait on stdin"
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("/help"),
        "the one turn ran: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// A completed turn exits zero.
#[test]
fn a_completed_turn_exits_zero() {
    let env = test_root("exit-zero");
    let (address, _bodies) = mock("all done");
    let path = env.root.join("instruction.md");
    std::fs::write(&path, "/help\n").unwrap();

    let mut args = base_args(&env);
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
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// A successful slash command may have no session-message side effect. Its
/// turn result must still be completed: exit status is the command outcome,
/// not an inference from `messages.len()` or `turns.len()`.
#[test]
fn a_successful_schema_turn_without_session_state_changes_exits_zero() {
    let env = test_root("schema-success");
    let (address, _bodies) = mock("unused");
    let path = env.root.join("instruction.md");
    std::fs::write(&path, "/schema\n").unwrap();

    let mut args = base_args(&env);
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
        Some(0),
        "a successful schema command completes the turn; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("\"event\":\"schema\""),
        "schema output should be rendered: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// An errored turn exits non-zero and the stream names the error: an
/// unknown slash command surfaces `TerminalEvent::Error` (stderr in every
/// format) carrying the command's name, so a harness can tell "the turn
/// failed" from "the turn said nothing".
#[test]
fn an_errored_turn_exits_nonzero_and_the_stream_names_the_error() {
    let env = test_root("errored");
    let (address, _bodies) = mock("unused");
    let path = env.root.join("instruction.md");
    std::fs::write(&path, "/no-such-command-xyz\n").unwrap();

    let mut args = base_args(&env);
    args.push("--turn-file".into());
    args.push(path.to_str().unwrap().into());
    let output = saya(
        &env,
        &address,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .output()
    .unwrap();
    assert_ne!(
        output.status.code(),
        Some(0),
        "an errored turn must exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains("no-such-command-xyz"),
        "the stream must name the error: {stderr}"
    );
    assert!(
        stderr.contains("\"event\":\"error\""),
        "the error rides the event stream: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// The flag changes nothing when absent: the piped path still reads
/// line-by-line (`/help` then `/exit`, exit 0) exactly as today.
#[test]
fn the_flag_changes_nothing_when_absent() {
    use std::io::Write as _;
    let env = test_root("absent");
    let (address, _bodies) = mock("unused");
    let args = base_args(&env);
    let mut child = {
        let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
        command
            .args(&args)
            .current_dir(&env.root)
            .env("SAYA_CONFIG_HOME", &env.root)
            .env("SAYA_SESSION_DIR", &env.sessions)
            .env("SAYA_STATE_DB", env.root.join("state.sqlite3"))
            .env("SAYA_PROVIDER", "openai_compatible")
            .env("SAYA_MODEL", "mock-model")
            .env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"))
            .env("SAYA_API_KEY", "mock-secret")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command.spawn().unwrap()
    };
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"/help\n/exit\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "piped path still exits zero; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("/help"),
        "piped /help still renders: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let _ = std::fs::remove_dir_all(&env.root);
}
