//! The `saya run` CLI surface, driven through the real binary (the
//! `tests/mvp` pattern): a scratch `SAYA_CONFIG_HOME`, `SAYA_RUNS_DIR`, and
//! `SAYA_STATE_DB` per test, and a scripted local HTTP provider standing in
//! for the model (`tests/mvp/provider.rs` is the recipe).
//!
//! Five guarantees:
//! 1. A headless `saya run` without `--allow` refuses before anything exists
//!    — no run directory, no prompt.
//! 2. A run paused by its budget exits 6, and `saya run resume <id>`
//!    continues it to completion.
//! 3. Ctrl-C cancels a run and exits 130.
//! 4. `saya run list` and `saya run show <id>` render a run that exists; an
//!    unknown id fails cleanly, never a panic.
//! 5. `--allow` with an unknown scope name is a usage error (2).

use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    process::{Command as ProcessCommand, Output, Stdio},
    sync::Arc,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};

/// One scripted provider response: its SSE body and how long to wait before
/// sending it (a wall-clock budget's trip depends on a slow response).
struct Scripted {
    body: String,
    delay_ms: u64,
}

/// A scripted local provider: one response per connection, in order; the
/// script spent, every further connection is accepted and dropped — a failed
/// call, never a hang. The thread never terminates on its own (it stays in
/// `accept`), so tests drop the handle rather than joining it: the thread
/// dies with the test process and can never block a test's exit. The
/// returned flag flips once the first connection has been read, so a test
/// can wait until the binary is provably inside a provider call.
fn mock(script: Vec<Scripted>) -> (String, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let first_arrived = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&first_arrived);
    thread::spawn(move || {
        let mut first = true;
        for scripted in script {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = [0_u8; 32_768];
                let _ = stream.read(&mut request);
                if first {
                    first = false;
                    flag.store(true, Ordering::SeqCst);
                }
                if scripted.delay_ms > 0 {
                    thread::sleep(std::time::Duration::from_millis(scripted.delay_ms));
                }
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: \
                     text/event-stream\r\nConnection: close\r\n\r\n{}",
                    scripted.body.len(),
                    scripted.body
                );
                let _ = stream.flush();
            }
        }
        while let Ok((stream, _)) = listener.accept() {
            drop(stream);
        }
    });
    (address, first_arrived)
}

/// Waits until the mock has served its first connection, with a generous
/// bound so a wedged child fails the test instead of hanging it.
fn wait_ready(flag: &Arc<AtomicBool>) {
    for _ in 0..200 {
        if flag.load(Ordering::SeqCst) {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("the binary never reached its first provider call");
}

/// One streaming response carrying `content` as the message's text.
fn sse(content: &str) -> String {
    let chunk = serde_json::json!({"choices": [{"delta": {"content": content}}]});
    format!("data: {chunk}\n\ndata: [DONE]\n\n")
}

/// The planner's answer: one JSON plan object, `steps` in order — the shape
/// `plan/prompt.rs` demands and the engine parses.
fn plan_body(goals: &[&str]) -> String {
    let steps = goals
        .iter()
        .map(|goal| {
            serde_json::json!({
                "goal": goal,
                "capabilities": {},
                "budget": serde_json::Value::Null,
                "expects": [],
                "endpoint": serde_json::Value::Null
            })
        })
        .collect::<Vec<_>>();
    let plan = serde_json::json!({"steps": steps});
    sse(&plan.to_string())
}

struct TestEnv {
    root: PathBuf,
    runs: PathBuf,
    state: PathBuf,
}

/// A per-test scratch root: config home, runs root, and state database all
/// live under one tree (`tests/mvp/common.rs` is the pattern).
fn test_root(label: &str) -> TestEnv {
    let root = std::env::temp_dir().join(format!("saya-run-cli-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    TestEnv {
        root: root.clone(),
        runs: root.join("runs"),
        state: root.join("state.sqlite3"),
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Runs the real `saya` binary against the scratch tree and `base_url`.
fn saya(env: &TestEnv, args: &[&str], address: &str) -> Output {
    ProcessCommand::new(env!("CARGO_BIN_EXE_saya"))
        .args(args)
        .current_dir(&env.root)
        .env("SAYA_CONFIG_HOME", env.root.join("user-config"))
        .env("SAYA_RUNS_DIR", &env.runs)
        .env("SAYA_STATE_DB", &env.state)
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"))
        .env("SAYA_API_KEY", "mock-secret")
        .output()
        .unwrap()
}

/// The newest run id from `saya run list`'s first line.
fn newest_run_id(output: &Output) -> String {
    stdout(output)
        .lines()
        .next()
        .unwrap_or_default()
        .split('\t')
        .next()
        .unwrap_or_default()
        .to_string()
}

/// A headless `saya run` without `--allow` refuses and exits non-zero, having
/// created no run directory. Assert the filesystem.
#[test]
fn headless_run_without_allow_refuses_before_anything_exists() {
    let env = test_root("refusal");
    // The provider is never contacted: the refusal precedes any assembly.
    let (address, _flag) = mock(Vec::new());
    let output = saya(
        &env,
        &["--non-interactive", "run", "answer the goal"],
        &address,
    );
    assert_eq!(
        output.status.code(),
        Some(2),
        "a headless run without --allow is a usage error; stderr: {}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("--allow"),
        "the refusal must say what is missing: {}",
        stderr(&output)
    );
    let created = match fs::read_dir(&env.runs) {
        Ok(entries) => entries.count(),
        Err(_) => 0,
    };
    assert_eq!(
        created,
        0,
        "a refused run must create no run directory: {:?}",
        fs::read_dir(&env.runs).map(|entries| entries
            .flatten()
            .map(|e| e.path().to_owned())
            .collect::<Vec<PathBuf>>())
    );
    let _ = fs::remove_dir_all(&env.root);
}

/// An unknown scope name is a usage error (2), not a silently ignored scope.
#[test]
fn allow_with_an_unknown_scope_is_a_usage_error() {
    let env = test_root("unknown-scope");
    let (address, _flag) = mock(Vec::new());
    let output = saya(
        &env,
        &["--non-interactive", "run", "--allow", "nonsense", "a goal"],
        &address,
    );
    assert_eq!(
        output.status.code(),
        Some(2),
        "an unknown scope is a usage error; stderr: {}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("unknown scope"),
        "the error must name the refused scope: {}",
        stderr(&output)
    );
    let _ = fs::remove_dir_all(&env.root);
}

/// A run paused by its budget exits 6, and `saya run resume <id>` continues
/// it. The budget is the wall clock: the plan binds fast, then the first
/// episode's response arrives after the ceiling, so the engine pauses —
/// never a silent incomplete run — and a resume with a fast provider
/// finishes the remaining step.
#[test]
fn a_run_paused_by_its_budget_exits_6_and_resume_continues_it() {
    let env = test_root("paused-budget");
    let (slow, _slow_ready) = mock(vec![
        Scripted {
            body: plan_body(&["survey the workspace", "finish the survey"]),
            delay_ms: 0,
        },
        Scripted {
            body: sse("step answer"),
            delay_ms: 2_500,
        },
    ]);
    let output = saya(
        &env,
        &[
            "--non-interactive",
            "run",
            "--allow",
            "workspace-write",
            "--budget",
            "wall-clock=1",
            "survey the data quality",
        ],
        &slow,
    );
    assert_eq!(
        output.status.code(),
        Some(6),
        "a run paused by its budget exits 6; stderr: {}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("paused"),
        "the pause must be said out loud: {}",
        stderr(&output)
    );
    let listing = saya(&env, &["run", "list"], &slow);
    assert_eq!(listing.status.code(), Some(0));
    let id = newest_run_id(&listing);
    assert!(!id.is_empty(), "the paused run must be listed: {id}");
    assert!(
        stdout(&listing).contains("paused"),
        "the listing must show the paused status: {}",
        stdout(&listing)
    );

    // The resume points at a fast provider: the run continues at the first
    // incomplete step and completes.
    let (fast, _fast_ready) = mock(vec![Scripted {
        body: sse("remaining step answer"),
        delay_ms: 0,
    }]);
    let resumed = saya(&env, &["run", "resume", &id], &fast);
    assert_eq!(
        resumed.status.code(),
        Some(0),
        "a resumed run must continue and complete; stderr: {}",
        stderr(&resumed)
    );
    let journal = fs::read_to_string(env.runs.join(&id).join("events.ndjson")).unwrap();
    assert!(
        journal.contains("\"completed\""),
        "the resumed run must record its completion: {journal}"
    );
    let _ = fs::remove_dir_all(&env.root);
}

/// Ctrl-C cancels a run: exit 130, the documented cancelled class.
#[test]
fn ctrl_c_cancels_a_run_with_exit_130() {
    let env = test_root("ctrl-c");
    // The plan call blocks far past the SIGINT: the child is inside the
    // provider call when the signal lands.
    let (address, ready) = mock(vec![Scripted {
        body: plan_body(&["never reached"]),
        delay_ms: 120_000,
    }]);
    let mut child = ProcessCommand::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "--non-interactive",
            "run",
            "--allow",
            "workspace-write",
            "a long goal",
        ])
        .current_dir(&env.root)
        .env("SAYA_CONFIG_HOME", env.root.join("user-config"))
        .env("SAYA_RUNS_DIR", &env.runs)
        .env("SAYA_STATE_DB", &env.state)
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"))
        .env("SAYA_API_KEY", "mock-secret")
        .spawn()
        .unwrap();
    // Wait until the child is provably inside its provider call — the
    // Ctrl-C handler is armed by then, because the select! that drives the
    // run registered it before the plan proposal started.
    wait_ready(&ready);
    // SAFETY: `kill` takes no pointers and has no invariants to uphold.
    let sent = unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGINT) };
    assert_eq!(sent, 0, "the child must be alive to receive SIGINT");
    let status = child.wait().unwrap();
    assert_eq!(
        status.code(),
        Some(130),
        "Ctrl-C must exit 130 through the handler, not die by signal"
    );
    let _ = fs::remove_dir_all(&env.root);
}

/// `saya run list` and `saya run show <id>` render a run that exists; `show`
/// on an unknown id fails cleanly rather than panicking.
#[test]
fn list_and_show_render_a_run_and_an_unknown_id_fails_cleanly() {
    let env = test_root("list-show");
    let (address, _ready) = mock(vec![
        Scripted {
            body: plan_body(&["survey the schema"]),
            delay_ms: 0,
        },
        Scripted {
            body: sse("survey complete"),
            delay_ms: 0,
        },
    ]);
    let started = saya(
        &env,
        &[
            "--non-interactive",
            "run",
            "--allow",
            "workspace-write",
            "survey the schema",
        ],
        &address,
    );
    assert_eq!(
        started.status.code(),
        Some(0),
        "the one-step run must complete; stderr: {}",
        stderr(&started)
    );
    let listing = saya(&env, &["run", "list"], &address);
    assert_eq!(listing.status.code(), Some(0));
    let id = newest_run_id(&listing);
    assert!(
        !id.is_empty(),
        "the completed run must be listed: {}",
        stdout(&listing)
    );
    let show = saya(&env, &["run", "show", &id], &address);
    assert_eq!(show.status.code(), Some(0), "stderr: {}", stderr(&show));
    let shown = stdout(&show);
    assert!(
        shown.contains("completed"),
        "the show must report the status: {shown}"
    );
    assert!(
        shown.contains("survey the schema"),
        "the show must report the run's goal: {shown}"
    );
    let log = saya(&env, &["run", "log", &id], &address);
    assert_eq!(log.status.code(), Some(0), "stderr: {}", stderr(&log));
    let logged = stdout(&log);
    assert!(
        logged.contains("run_started") && logged.contains("plan_approved"),
        "the log must carry the journal events: {logged}"
    );
    // An unknown id fails cleanly: non-zero, a message that names the miss,
    // and never a panic (101).
    let unknown = saya(&env, &["run", "show", "r-nope"], &address);
    assert_ne!(
        unknown.status.code(),
        Some(101),
        "an unknown run id must not panic; stderr: {}",
        stderr(&unknown)
    );
    assert_ne!(
        unknown.status.code(),
        Some(0),
        "an unknown run id must fail"
    );
    assert!(
        stderr(&unknown).contains("no run with id"),
        "the failure must say why: {}",
        stderr(&unknown)
    );
    let _ = fs::remove_dir_all(&env.root);
}

// ---------------------------------------------------------------------------
// The run wire (NDJSON): lifecycle `RunEvent` lines tagged "type", interleaved
// with the episode events in today's `TerminalEvent` envelope tagged "event".
// The stream is the journal itself — every line is one JSON object, and the
// Spider benchmark harness (which reads `event` keys) sees nothing new to
// trip over.
// ---------------------------------------------------------------------------

/// Parses `text` as one JSON object per line, every line, and returns the
/// objects. A blank line is tolerated (an empty stream); a non-blank line
/// that does not parse fails the test.
fn json_lines(text: &str, label: &str) -> Vec<serde_json::Value> {
    let mut objects = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|error| {
            panic!("{label} line is not one JSON object: {line:?}: {error}")
        });
        assert!(
            value.is_object(),
            "{label} line must be one JSON object: {line:?}"
        );
        objects.push(value);
    }
    objects
}

/// Collects the `"type"` tags of the streamed `RunEvent` lines.
fn type_tags(values: &[serde_json::Value]) -> Vec<&str> {
    values
        .iter()
        .filter_map(|value| value.get("type").and_then(|tag| tag.as_str()))
        .collect()
}

/// A paused run's NDJSON stream is one JSON object per line, every line —
/// lifecycle lines tagged `"type"`, interleaved with the episode's events in
/// the `TerminalEvent` envelope tagged `"event"`. The pause lands as a
/// `paused` line naming its reason, and the terminal settle message is a JSON
/// error line on stderr, never stray text.
#[test]
fn an_ndjson_run_stream_parses_as_one_json_object_per_line_when_paused() {
    let env = test_root("ndjson-pause");
    let (slow, _ready) = mock(vec![
        Scripted {
            body: plan_body(&["survey the workspace"]),
            delay_ms: 0,
        },
        Scripted {
            body: sse("step answer"),
            delay_ms: 2_500,
        },
    ]);
    let output = saya(
        &env,
        &[
            "--format",
            "ndjson",
            "--non-interactive",
            "run",
            "--allow",
            "workspace-write",
            "--budget",
            "wall-clock=1",
            "survey the data quality",
        ],
        &slow,
    );
    assert_eq!(
        output.status.code(),
        Some(6),
        "the budget trips and the run pauses; stderr: {}",
        stderr(&output)
    );

    // Every line, both streams, is one JSON object — nothing else is ever
    // printed on the wire.
    let out = json_lines(&stdout(&output), "stdout");
    let _err = json_lines(&stderr(&output), "stderr");

    // The lifecycle rode the wire in journal order.
    let tags = type_tags(&out);
    for expected in ["run_started", "plan_approved", "step_started", "paused"] {
        assert!(
            tags.contains(&expected),
            "the stream must carry a {expected} line: {out:?}"
        );
    }
    // The episode's events are present under today's envelope — the two tags
    // share one stream by design, and the dual-tag stream stays one
    // JSON object per line either way.
    assert!(
        out.iter()
            .any(|value| value.get("event").and_then(|tag| tag.as_str()).is_some()),
        "the episode events ride the TerminalEvent envelope: {out:?}"
    );
    let paused = out
        .iter()
        .find(|value| value.get("type").and_then(|tag| tag.as_str()) == Some("paused"))
        .expect("a paused line");
    assert_eq!(
        paused.get("reason").and_then(|reason| reason.as_str()),
        Some("wall_clock_exceeded"),
        "the pause names its cause: {paused:?}"
    );

    // The resume speaks the same wire: every line is one JSON object, and the
    // run it continues ends `completed` on the same stream.
    let id = {
        let listing = saya(&env, &["run", "list"], &slow);
        newest_run_id(&listing)
    };
    let (fast, _fast_ready) = mock(vec![Scripted {
        body: sse("remaining step answer"),
        delay_ms: 0,
    }]);
    let resumed = saya(&env, &["--format", "ndjson", "run", "resume", &id], &fast);
    assert_eq!(
        resumed.status.code(),
        Some(0),
        "the resumed run completes; stderr: {}",
        stderr(&resumed)
    );
    let resumed_lines = json_lines(&stdout(&resumed), "resumed stdout");
    json_lines(&stderr(&resumed), "resumed stderr");
    let resumed_tags = type_tags(&resumed_lines);
    // The pause landed mid-episode, so the journal's step is already
    // complete when the resume replays it: resume records the death pause and
    // the completion the crash had left unwritten — the machine's own story.
    assert!(
        resumed_tags.contains(&"paused") && resumed_tags.contains(&"completed"),
        "the resume carries its lifecycle to completion: {resumed_lines:?}"
    );
    let _ = fs::remove_dir_all(&env.root);
}

/// A failing run's stream is one JSON object per line too: the bounded
/// retries surface as step_failed lines, the pause is said out loud, and the
/// failure message is a JSON error line on stderr — never stray text that a
/// line-oriented consumer would choke on.
#[test]
fn an_ndjson_run_stream_stays_line_oriented_when_the_run_fails() {
    let env = test_root("ndjson-fail");
    // The plan binds, then every episode call fails: the mock has spent its
    // script, so each further connection is accepted and dropped — a provider
    // failure, never a hang. The engine retries the step bounded, then pauses.
    let (failing, _ready) = mock(vec![Scripted {
        body: plan_body(&["a step that cannot succeed"]),
        delay_ms: 0,
    }]);
    let output = saya(
        &env,
        &[
            "--format",
            "ndjson",
            "--non-interactive",
            "run",
            "--allow",
            "workspace-write",
            "a goal whose steps all fail",
        ],
        &failing,
    );
    assert_eq!(
        output.status.code(),
        Some(6),
        "the spent retries pause the run; stderr: {}",
        stderr(&output)
    );
    let out = json_lines(&stdout(&output), "stdout");
    let _err = json_lines(&stderr(&output), "stderr");
    let tags = type_tags(&out);
    assert!(
        tags.iter().filter(|tag| **tag == "step_failed").count() >= 3,
        "the bounded retries are on the wire: {out:?}"
    );
    let paused = out
        .iter()
        .find(|value| value.get("type").and_then(|tag| tag.as_str()) == Some("paused"))
        .expect("a paused line");
    assert_eq!(
        paused.get("reason").and_then(|reason| reason.as_str()),
        Some("step_failed_after_retry"),
        "the pause names the spent retries: {paused:?}"
    );
    let _ = fs::remove_dir_all(&env.root);
}

/// A nested `saya run` started from a session `/run` passes its whole stream
/// through the parent's stdout unmangled: the parent never re-tags a child
/// line into its own envelope and never swallows one. The exact child line
/// (`{"type":"run_started"}`) appears in the parent's stdout byte-for-byte.
#[test]
fn a_nested_run_child_lines_survive_to_the_parent_stdout_unmangled() {
    let env = test_root("nested-child");
    let (address, _ready) = mock(vec![
        Scripted {
            body: plan_body(&["survey the schema"]),
            delay_ms: 0,
        },
        Scripted {
            body: sse("survey complete"),
            delay_ms: 0,
        },
    ]);
    // The parent session, piped: one `/run` line in NDJSON mode. The child
    // inherits the parent's env, so the run lands in the same scratch tree.
    let mut parent = ProcessCommand::new(env!("CARGO_BIN_EXE_saya"))
        .args(["--format", "ndjson"])
        .current_dir(&env.root)
        .env("SAYA_CONFIG_HOME", env.root.join("user-config"))
        .env("SAYA_RUNS_DIR", &env.runs)
        .env("SAYA_STATE_DB", &env.state)
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"))
        .env("SAYA_API_KEY", "mock-secret")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    parent
        .stdin
        .take()
        .unwrap()
        .write_all(b"/run survey the data --allow workspace-write\n")
        .unwrap();
    let output = parent.wait_with_output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "the session survives the nested run; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let parent_stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    // The child's own first line, byte for byte — not wrapped, re-tagged, or
    // dropped: the parent re-emits nothing, it passed the file descriptors
    // through.
    let run_started = format!("{}\n", serde_json::json!({"type": "run_started"}));
    assert!(
        parent_stdout.contains(&run_started),
        "the child's run_started line must survive unmangled: {parent_stdout:?}"
    );
    // And every line the parent emitted is still one JSON object per line —
    // the child's stream is a valid stream inside the parent's.
    let values: Vec<serde_json::Value> = parent_stdout
        .lines()
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|error| panic!("parent line not JSON: {line:?}: {error}"))
        })
        .collect();
    assert!(
        values
            .iter()
            .any(|value| value.get("type") == Some(&serde_json::json!("completed"))),
        "the child's run completed on the parent's stream: {values:?}"
    );

    // The run the child made is the one the session's /runs can see: same
    // runs root, same store.
    let listing = saya(&env, &["run", "list"], &address);
    assert!(
        stdout(&listing).contains("completed"),
        "the nested run is visible to `saya run list`: {}",
        stdout(&listing)
    );
    let _ = fs::remove_dir_all(&env.root);
}
