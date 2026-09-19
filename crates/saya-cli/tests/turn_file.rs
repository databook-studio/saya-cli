//! H-S1 `--turn-file`: one verbatim turn, then exit.
//!
//! The only headless path into a session today is piped stdin read
//! line-by-line (`session_loop.rs`), which folds multi-line markdown.
//! These tests pin the new flag: the file bytes reach the turn unaltered,
//! exactly one turn runs, the process exits, and the exit code reflects the
//! turn's outcome. The last test pins today's piped path unchanged.

use std::{
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

struct TestEnv {
    root: PathBuf,
    connections: PathBuf,
    sessions: PathBuf,
}

fn test_root(name: &str) -> TestEnv {
    let root = std::env::temp_dir().join(format!(
        "saya-cli-turn-file-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("turn.duckdb");
    duckdb::Connection::open(&database)
        .unwrap()
        .execute_batch("CREATE TABLE t (id INTEGER); INSERT INTO t VALUES (1);")
        .unwrap();
    let connections = root.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'duckdb'\npath = '{}'\nread_only = true\n",
            database.display()
        ),
    )
    .unwrap();
    let sessions = root.join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    TestEnv {
        root,
        connections,
        sessions,
    }
}

/// Mock OpenAI-compatible SSE endpoint. Captures the first request body
/// verbatim and answers every round with `answer`; returns the listener
/// address plus the captured bodies.
fn mock(answer: &str) -> (String, std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let bodies = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Vec<u8>>::new()));
    let seen = std::sync::Arc::clone(&bodies);
    let answer = answer.to_owned();
    thread::spawn(move || {
        for _ in 0..16 {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut raw = Vec::new();
            let mut buf = [0_u8; 8192];
            loop {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(size) => raw.extend_from_slice(&buf[..size]),
                }
            }
            seen.lock().unwrap().push(raw);
            let chunk = serde_json::json!({"choices": [{"delta": {"content": answer}}]});
            let body = format!("data: {chunk}\n\ndata: [DONE]\n\n");
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.flush();
        }
    });
    (address, bodies)
}

fn saya(env: &TestEnv, address: &str, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .args(args)
        .current_dir(&env.root)
        .env("SAYA_CONFIG_HOME", &env.root)
        .env("SAYA_SESSION_DIR", &env.sessions)
        .env("SAYA_STATE_DB", env.root.join("state.sqlite3"))
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"))
        .env("SAYA_API_KEY", "mock-secret")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn base_args(env: &TestEnv) -> Vec<String> {
    vec![
        "--approval-mode".into(),
        "read-only".into(),
        "--allow-data-sharing".into(),
        "--format".into(),
        "ndjson".into(),
        "--connections".into(),
        env.connections.to_str().unwrap().into(),
        "--profile".into(),
        "local".into(),
    ]
}

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

/// Startup notices stay off the machine-readable stdout stream: every line
/// there must remain valid NDJSON, while the human bypass explanation remains
/// visible on stderr.
#[test]
fn bypass_activation_notice_does_not_corrupt_ndjson_stdout() {
    use std::io::Write as _;

    let env = test_root("bypass-ndjson");
    let (address, _bodies) = mock("unused");
    let mut args = base_args(&env);
    args[1] = "bypass".into();
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
    let mut child = command.spawn().unwrap();
    child.stdin.as_mut().unwrap().write_all(b"/exit\n").unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0));

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines().filter(|line| !line.is_empty()) {
        serde_json::from_str::<serde_json::Value>(line)
            .unwrap_or_else(|error| panic!("stdout line is not NDJSON: {line:?}: {error}"));
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("bypass on: every tool call runs without asking"));
    let _ = std::fs::remove_dir_all(&env.root);
}

/// The flag is session-surface only: `saya ask` and `saya run` refuse it
/// rather than silently ignoring a stated intent.
#[test]
fn turn_file_is_refused_by_ask_and_run() {
    let env = test_root("refused");
    let (address, _bodies) = mock("unused");
    for subcommand in [vec!["ask", "hi"], vec!["run", "--allow", "none", "goal"]] {
        let mut args = vec![
            "--connections",
            env.connections.to_str().unwrap(),
            "--profile",
            "local",
        ];
        args.extend(subcommand.iter().copied());
        args.extend(["--turn-file", "instruction.md"]);
        let output = saya(&env, &address, &args).output().unwrap();
        assert_ne!(
            output.status.code(),
            Some(0),
            "{subcommand:?} must refuse --turn-file"
        );
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        assert!(
            stderr.contains("--turn-file"),
            "the refusal names the flag: {stderr}"
        );
    }
    let _ = std::fs::remove_dir_all(&env.root);
}
