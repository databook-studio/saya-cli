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

/// `/sessions list` and `/sessions delete <id>` through the real headless
/// binary: the slash forms drive the same shared saved-session operations
/// `saya sessions` uses. The listing names a second saved session written
/// directly into the scratch store and ends with the storage root; a delete
/// exits zero, prints the retained-data sentence, removes exactly that
/// session's record and keeps the live session's own; a delete of an
/// unknown id exits non-zero saying so. stdin is null throughout — the
/// forms never prompt.
#[test]
fn turn_file_sessions_list_and_delete_drive_the_shared_operations() {
    const RETAINED_NOTE: &str = "Project preferences and typed database knowledge are stored \
                                 separately and were not deleted.";
    let env = test_root("sessions-slash");
    let (address, _bodies) = mock("unused");
    // A second saved session in the scratch store, as a minimal
    // legacy-shaped record the store accepts.
    let other = env.sessions.join("other-session.json");
    std::fs::write(&other, r#"{"id":"other-session","turns":[],"messages":[]}"#).unwrap();
    let run = |line: &str| {
        let path = env.root.join("instruction.md");
        std::fs::write(&path, format!("{line}\n")).unwrap();
        let mut args = base_args(&env);
        args.push("--turn-file".into());
        args.push(path.to_str().unwrap().into());
        saya(
            &env,
            &address,
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
        )
        .output()
        .unwrap()
    };

    // An unknown id: non-zero, the refusal names the id, nothing is removed.
    let output = run("/sessions delete nosuch-session-0001");
    assert_ne!(
        output.status.code(),
        Some(0),
        "an unknown-id delete must exit non-zero; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains("no saved session nosuch-session-0001"),
        "the refusal names the id: {stderr}"
    );
    assert!(other.exists(), "an unknown-id delete removes nothing");

    // The listing: exit zero, the saved session is named, the storage root
    // is the last line.
    let output = run("/sessions list");
    assert_eq!(
        output.status.code(),
        Some(0),
        "the listing completes; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        stdout.contains("other-session"),
        "the saved session is listed: {stdout}"
    );
    assert!(
        stdout.contains("Storage root:"),
        "the listing ends with the storage root: {stdout}"
    );

    // The delete: exit zero, the retained-data sentence, the record gone,
    // the live session's own record kept.
    let output = run("/sessions delete other-session");
    assert_eq!(
        output.status.code(),
        Some(0),
        "the delete completes; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        stdout.contains("Deleted saved session other-session"),
        "the success names the id: {stdout}"
    );
    assert!(
        stdout.contains(RETAINED_NOTE),
        "the retained-data sentence is said: {stdout}"
    );
    assert!(!other.exists(), "the deleted session's record is gone");
    let remaining: Vec<String> = std::fs::read_dir(&env.sessions)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    assert!(
        !remaining.is_empty() && remaining.iter().all(|name| name != "other-session.json"),
        "the current session's own record survives and no other-session record remains: \
         {remaining:?}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// The first `wt-…` worktree label in a listing — 67 chars, `wt-` plus 64
/// lowercase hex.
fn first_worktree_label(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|token| token.len() == 67 && token.starts_with("wt-"))
        .map(str::to_owned)
}

/// `/preferences` through the real headless binary: the slash forms drive
/// the same shared operations `saya preferences` uses, scoped to the
/// SESSION's bound worktree root (`--workspace`), never the launch cwd —
/// and the worktree label the session shows is the one the headless CLI
/// shows for the same root (one identity derivation). Add exits zero, the
/// listing carries the text with a `pp-` id, the forget removes it, and an
/// unknown subcommand exits non-zero naming the usage. One turn per run;
/// stdin is null throughout — the forms never prompt.
#[test]
fn turn_file_preferences_slash_forms_drive_the_shared_operations() {
    let env = test_root("preferences-slash");
    let (address, _bodies) = mock("unused");
    // A dedicated subdir, sibling to `env.sessions` — `--workspace` must
    // not contain the session store (a separate, unrelated containment
    // guard refuses that). It holds a `.git` dir, so the CLI's walk-up
    // resolves the same canonical root the session binds explicitly.
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(workspace.join(".git")).unwrap();
    let run = |line: &str| {
        let path = env.root.join("instruction.md");
        std::fs::write(&path, format!("{line}\n")).unwrap();
        let mut args = base_args(&env);
        args.push("--workspace".into());
        args.push(workspace.to_str().unwrap().into());
        args.push("--turn-file".into());
        args.push(path.to_str().unwrap().into());
        saya(
            &env,
            &address,
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
        )
        .output()
        .unwrap()
    };

    // Run 1: the add through the session surface exits zero.
    let added = run("/preferences add prefer tabs over spaces");
    assert_eq!(
        added.status.code(),
        Some(0),
        "the add completes; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&added.stdout),
        String::from_utf8_lossy(&added.stderr)
    );
    let added_out = String::from_utf8_lossy(&added.stdout).into_owned();
    assert!(
        added_out.contains("Added preference pp-"),
        "the add names the id: {added_out}"
    );

    // Run 2: the listing shows the text, a `pp-` id, and the same
    // worktree label the headless CLI shows for the same root.
    let listing = run("/preferences list");
    assert_eq!(
        listing.status.code(),
        Some(0),
        "the listing completes; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&listing.stdout),
        String::from_utf8_lossy(&listing.stderr)
    );
    let listing_out = String::from_utf8_lossy(&listing.stdout).into_owned();
    assert!(
        listing_out.contains("prefer tabs over spaces"),
        "the preference is listed: {listing_out}"
    );
    // The ndjson `result` event's message is the listing itself, unescaped
    // — tokens are split on the real message, not on the JSON encoding.
    let event_message = |stdout: &str| -> String {
        stdout
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter_map(|value| {
                value
                    .get("message")
                    .and_then(|message| message.as_str().map(str::to_owned))
            })
            .next()
            .unwrap_or_default()
    };
    let listing_message = event_message(&listing_out);
    let session_id = listing_message
        .split_whitespace()
        .find(|token| token.len() == 35 && token.starts_with("pp-"))
        .expect("the listing carries a pp- id")
        .to_owned();
    let session_label =
        first_worktree_label(&listing_message).expect("the listing names the worktree");
    let cli_listing = {
        // The headless subcommand from the same workspace cwd, against the
        // same state db the `saya` helper isolates into the scratch root.
        let output = Command::new(env!("CARGO_BIN_EXE_saya"))
            .args(["preferences", "list"])
            .current_dir(&workspace)
            .env("SAYA_STATE_DB", env.root.join("state.sqlite3"))
            .env("SAYA_CONFIG_HOME", &env.root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(0),
            "the CLI listing completes: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    let cli_label = first_worktree_label(&cli_listing).expect("the CLI listing names the worktree");
    assert_eq!(
        session_label, cli_label,
        "one identity derivation: the session and the CLI name the same worktree"
    );

    // Run 3: the forget removes it; the listing is empty again.
    let forgotten = run(&format!("/preferences forget {session_id}"));
    assert_eq!(
        forgotten.status.code(),
        Some(0),
        "the forget completes; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&forgotten.stdout),
        String::from_utf8_lossy(&forgotten.stderr)
    );
    assert!(
        String::from_utf8_lossy(&forgotten.stdout).contains("Forgot preference"),
        "{}",
        String::from_utf8_lossy(&forgotten.stdout)
    );
    let empty = run("/preferences list");
    assert!(
        String::from_utf8_lossy(&empty.stdout).contains("No project preferences for"),
        "{}",
        String::from_utf8_lossy(&empty.stdout)
    );

    // An unknown subcommand exits non-zero naming the usage.
    let unknown = run("/preferences nope");
    assert_ne!(
        unknown.status.code(),
        Some(0),
        "an unknown subcommand is a usage error; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&unknown.stdout),
        String::from_utf8_lossy(&unknown.stderr)
    );
    let stderr = String::from_utf8_lossy(&unknown.stderr).into_owned();
    assert!(
        stderr.contains("unknown /preferences subcommand: nope"),
        "the refusal names the offence and the usage: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// `/notes` through the real headless binary: bare `/notes` says there are
/// no session notes (exit zero, the remedy named), and `/notes set <text>`
/// refuses with the /compact-first remedy and exits non-zero — a
/// continuation record exists only after a compaction, and `--turn-file`
/// runs a fresh session per turn on the same session dir, so both runs are
/// the empty/refusal shapes.
#[test]
fn turn_file_notes_show_and_set_refuse_without_a_continuation() {
    let env = test_root("notes-slash");
    let (address, _bodies) = mock("unused");
    let run = |line: &str| {
        let path = env.root.join("instruction.md");
        std::fs::write(&path, format!("{line}\n")).unwrap();
        let mut args = base_args(&env);
        args.push("--turn-file".into());
        args.push(path.to_str().unwrap().into());
        saya(
            &env,
            &address,
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
        )
        .output()
        .unwrap()
    };

    // Bare `/notes`: exit zero, the no-notes words with the remedy.
    let shown = run("/notes");
    assert_eq!(
        shown.status.code(),
        Some(0),
        "the show completes; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&shown.stdout),
        String::from_utf8_lossy(&shown.stderr)
    );
    let stdout = String::from_utf8_lossy(&shown.stdout).into_owned();
    assert!(
        stdout.contains("No session notes") && stdout.contains("/notes set <text>"),
        "the show names the remedy: {stdout}"
    );

    // `/notes set x`: exit non-zero, the /compact-first refusal.
    let refused = run("/notes set remember the failing query");
    assert_ne!(
        refused.status.code(),
        Some(0),
        "a set with no continuation record must exit non-zero; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&refused.stdout),
        String::from_utf8_lossy(&refused.stderr)
    );
    let stderr = String::from_utf8_lossy(&refused.stderr).into_owned();
    assert!(
        stderr.contains("run /compact first"),
        "the refusal names the /compact remedy: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// `/rotate` through the real headless binary: a `--turn-file` run starts a
/// fresh session, so the rotation always refuses with the /compact-first
/// remedy — exit 5, the streamed error names it, no provider round runs,
/// and stdout stays valid NDJSON. The success path is covered by the
/// `session_rotate_run` unit tests against a real store and runtime.
#[test]
fn turn_file_rotate_refuses_a_fresh_session_and_exits_five() {
    let env = test_root("rotate-slash");
    let (address, bodies) = mock("unused");
    let path = env.root.join("instruction.md");
    std::fs::write(&path, "/rotate\n").unwrap();
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
        Some(5),
        "the refusal exits 5; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains("run /compact first"),
        "the refusal names the /compact remedy: {stderr}"
    );
    assert!(
        bodies.lock().unwrap().is_empty(),
        "a slash command runs no provider round"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines().filter(|line| !line.is_empty()) {
        serde_json::from_str::<serde_json::Value>(line)
            .unwrap_or_else(|error| panic!("stdout line is not NDJSON: {line:?}: {error}"));
    }
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

// ---------------------------------------------------------------------------
// launch `--allow` seeds every scope, never only `command:` — the relay
// bug: `session_loop.rs` used to feed the shared grammar-then-composition
// gate (`seed_launch_allow`) a pre-filtered `command:`-only copy of the
// launch's `--allow` tokens, so every other scope type (`workspace-write`,
// `sql:`, `runner:`, `interpreter:`, `scratch`, `fetch:`) seeded nothing
// and said nothing — a silent no-op. These pin the fix end to end, through
// the real launch path both surfaces share.
// ---------------------------------------------------------------------------

/// A launch `--allow workspace-write` seed lands in the grant store:
/// `/grants` on the one turn names it.
#[test]
fn a_launch_workspace_write_seed_is_granted() {
    let env = test_root("launch-workspace-write");
    let (address, _bodies) = mock("unused");
    let path = env.root.join("instruction.md");
    std::fs::write(&path, "/grants\n").unwrap();
    // A dedicated subdir, sibling to `env.sessions` — `--workspace` must
    // not contain the session store (a separate, unrelated containment
    // guard refuses that).
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    let mut args = base_args(&env);
    args.push("--workspace".into());
    args.push(workspace.to_str().unwrap().into());
    args.push("--allow".into());
    args.push("workspace-write".into());
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
        "a carriable launch seed starts the session; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("workspace-write"),
        "the launch seed is granted: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// A launch `--allow sql:<connection>` seed lands in the grant store —
/// `sql:` gates nothing at composition (`allow_refusal` has no arm for it),
/// so no workspace binding is needed for it to seed.
#[test]
fn a_launch_sql_seed_is_granted() {
    let env = test_root("launch-sql");
    let (address, _bodies) = mock("unused");
    let path = env.root.join("instruction.md");
    std::fs::write(&path, "/grants\n").unwrap();

    let mut args = base_args(&env);
    args.push("--allow".into());
    args.push("sql:local".into());
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
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("sql:local"),
        "the launch seed is granted: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// A launch `--allow runner:bench` seed with no `[jobs.runner]` composed is
/// a launch usage error — the same reason `/allow` gives mid-session — and
/// the process refuses to start rather than silently dropping the seed.
#[test]
fn a_launch_runner_seed_without_a_runner_is_a_usage_error() {
    let env = test_root("launch-runner-no-runner");
    let (address, _bodies) = mock("unused");
    let path = env.root.join("instruction.md");
    std::fs::write(&path, "/grants\n").unwrap();

    let mut args = base_args(&env);
    args.push("--allow".into());
    args.push("runner:bench".into());
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
        "a seed the composition cannot carry refuses to start: stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains("invalid --allow seed") && stderr.contains("composed no runner"),
        "the refusal names the gap: {stderr}"
    );
    assert!(
        stderr.contains("run_command") && stderr.contains("ask for approval"),
        "the refusal names the fallback: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// A launch `--allow runner:python3` seed is a grammar-level usage error
/// (`python3` is a known interpreter, refused for the `runner:` family
/// before composition is ever consulted) — also refused at launch, never
/// silently dropped.
#[test]
fn a_launch_grammar_error_is_a_usage_error() {
    let env = test_root("launch-grammar-error");
    let (address, _bodies) = mock("unused");
    let path = env.root.join("instruction.md");
    std::fs::write(&path, "/grants\n").unwrap();

    let mut args = base_args(&env);
    args.push("--allow".into());
    args.push("runner:python3".into());
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
        "a grammar-refused seed refuses to start: stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains("shell or interpreter") && stderr.contains("interpreter:python3"),
        "the refusal names the grammar fix: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// Regression: a launch `--allow command:git` seed still works after the
/// relay fix — granted exactly once, journalled exactly once (the previous
/// relay granted `command:` tokens through two separate paths).
#[cfg(unix)]
#[test]
fn a_launch_command_seed_still_works() {
    let env = test_root("launch-command");
    let (address, _bodies) = mock("unused");
    let path = env.root.join("instruction.md");
    std::fs::write(&path, "/grants\n").unwrap();
    // A dedicated subdir, sibling to `env.sessions` — `--workspace` must
    // not contain the session store (a separate, unrelated containment
    // guard refuses that).
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    let mut args = base_args(&env);
    args.push("--workspace".into());
    args.push(workspace.to_str().unwrap().into());
    args.push("--allow".into());
    args.push("command:git".into());
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
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("command:git"),
        "the launch seed is granted: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    // Exactly one session directory exists (a fresh session per run); its
    // journal carries exactly one `Granted` line for the token, never two.
    let mut sessions = std::fs::read_dir(&env.sessions)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir());
    let session_dir = sessions.next().expect("the run created one session dir");
    assert!(
        sessions.next().is_none(),
        "exactly one session directory from this run"
    );
    let journal = std::fs::read_to_string(session_dir.path().join("journal.ndjson")).unwrap();
    let granted_lines = journal
        .lines()
        .filter(|line| line.contains("command:git") && line.contains("session-granted"))
        .count();
    assert_eq!(
        granted_lines, 1,
        "the token is journalled exactly once, never doubled: {journal}"
    );
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

/// `/context` through the real headless binary: the workspace-root AGENTS.md
/// and the personal instructions.md are named with their receipts (size,
/// digest, truncation), and neither body text ever reaches stdout.
#[test]
fn a_turn_file_context_reports_sources_without_their_bodies() {
    const PROJECT_SENTINEL: &str = "SENTINEL-INSTRUCTION-BODY";
    const PERSONAL_SENTINEL: &str = "SENTINEL-PERSONAL-INSTRUCTION-BODY";
    let env = test_root("turn-file-context");
    let (address, _bodies) = mock("unused");
    let path = env.root.join("instruction.md");
    std::fs::write(&path, "/context\n").unwrap();
    // A dedicated subdir, sibling to `env.sessions` — `--workspace` must
    // not contain the session store (a separate, unrelated containment
    // guard refuses that).
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let project_body = format!("project rules with {PROJECT_SENTINEL}\n");
    std::fs::write(workspace.join("AGENTS.md"), &project_body).unwrap();
    // The scratch config home is `env.root` (see `saya`), so the personal
    // instructions file lives at `<config home>/saya/instructions.md`.
    let personal_body = format!("personal rules with {PERSONAL_SENTINEL}\n");
    std::fs::create_dir_all(env.root.join("saya")).unwrap();
    std::fs::write(env.root.join("saya/instructions.md"), &personal_body).unwrap();

    let mut args = base_args(&env);
    args.push("--workspace".into());
    args.push(workspace.to_str().unwrap().into());
    args.push("--turn-file".into());
    args.push(path.to_str().unwrap().into());
    let output = saya(
        &env,
        &address,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .output()
    .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(
        output.status.code(),
        Some(0),
        "the /context turn completes: stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("AGENTS.md"),
        "the project source is named: {stdout}"
    );
    assert!(
        stdout.contains("instructions.md"),
        "the personal source is named: {stdout}"
    );
    assert!(
        stdout.contains(&format!("{} B file", project_body.len())),
        "the whole-file size is shown: {stdout}"
    );
    assert!(
        !stdout.contains(PROJECT_SENTINEL),
        "the project body must never appear: {stdout}"
    );
    assert!(
        !stdout.contains(PERSONAL_SENTINEL),
        "the personal body must never appear: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}
