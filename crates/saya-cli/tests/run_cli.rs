//! The `saya run` CLI surface, driven through the real binary (the
//! `tests/mvp` pattern): a scratch `SAYA_CONFIG_HOME`, `SAYA_RUNS_DIR`, and
//! `SAYA_STATE_DB` per test, and a scripted local HTTP provider standing in
//! for the model (`tests/mvp/provider.rs` is the recipe).
//!
//! Seven guarantees:
//! 1. A headless `saya run` without `--allow` refuses before anything exists
//!    — no run directory, no prompt.
//! 2. A run paused by its budget exits 6, and `saya run resume <id>`
//!    continues it to completion.
//! 3. Ctrl-C cancels a run and exits 130.
//! 4. `saya run list` and `saya run show <id>` render a run that exists; an
//!    unknown id fails cleanly, never a panic.
//! 5. `--allow` with an unknown scope name is a usage error (2).
//! 6. A plan asking for more than `--allow` granted is refused, naming the
//!    missing scope.
//! 7. On a real terminal the bound plan is approved once and the run
//!    proceeds — no further interaction; a refusal refuses with exit 2.

use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    process::{Command as ProcessCommand, Output},
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

/// The planner's answer with per-step capabilities, for plans that ask for
/// scopes beyond what `--allow` granted.
fn plan_body_with_capabilities(steps: &[(&str, serde_json::Value)]) -> String {
    let steps = steps
        .iter()
        .map(|(goal, capabilities)| {
            serde_json::json!({
                "goal": goal,
                "capabilities": capabilities,
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

/// A plan whose steps ask for more than `--allow` granted is rejected with a
/// usage refusal that names the missing scope and its step — the model
/// cannot fix that by re-planning, and a headless run cannot ask. The
/// engine re-prompts to its bound of three, so the mock serves three plans.
#[test]
fn a_plan_asking_for_more_than_allow_granted_is_refused_naming_the_scope() {
    let env = test_root("needs-approval");
    let asking = plan_body_with_capabilities(&[(
        "seed the scratch database",
        serde_json::json!({"scratch": true}),
    )]);
    let (address, _ready) = mock(vec![
        Scripted {
            body: asking.clone(),
            delay_ms: 0,
        },
        Scripted {
            body: asking.clone(),
            delay_ms: 0,
        },
        Scripted {
            body: asking,
            delay_ms: 0,
        },
    ]);
    let output = saya(
        &env,
        &[
            "--non-interactive",
            "run",
            "--allow",
            "workspace-write",
            "seed and report",
        ],
        &address,
    );
    assert_eq!(
        output.status.code(),
        Some(2),
        "a plan asking beyond --allow is a usage refusal; stderr: {}",
        stderr(&output)
    );
    let message = stderr(&output);
    assert!(
        message.contains("scratch"),
        "the refusal must name the missing scope: {message}"
    );
    assert!(
        message.contains("step 0"),
        "the refusal must point at the asking step: {message}"
    );
    assert!(
        message.contains("--allow"),
        "the refusal must say the lever to widen: {message}"
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
// Interactive approval on a real terminal: the binary's stdin is a PTY, so
// `can_prompt` holds and the bound plan is approved through the channel —
// the `tui/agent.rs` pattern with the terminal answering. The approval view
// is the one interaction; the run then proceeds without asking again.
// ---------------------------------------------------------------------------

/// What one interactive run leaves behind: the exit code and the captured
/// terminal stream, for assertions on both the prompt and the completion.
struct InteractiveRun {
    exit_code: Option<i32>,
    stream: String,
}

/// Spawns `saya run` on a PTY, waits for the approval prompt to appear,
/// answers with `answer`, and waits for the child to exit. A timeout kills
/// the child and fails the test — an interactive ask that never resolves is
/// a hang, not a pass.
fn run_interactively(env: &TestEnv, args: &[&str], address: &str, answer: &str) -> InteractiveRun {
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};
    const DEADLINE: Duration = Duration::from_secs(30);
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("the pty must be allocatable");
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_saya"));
    // The headless helper runs the child in the scratch root; on a PTY the
    // builder would otherwise fall back to $HOME as the cwd, where a real
    // project-layer `.saya/config.toml` would shadow the scratch tree.
    cmd.cwd(&env.root);
    cmd.args(args);
    cmd.env("SAYA_CONFIG_HOME", env.root.join("user-config"));
    cmd.env("SAYA_RUNS_DIR", &env.runs);
    cmd.env("SAYA_STATE_DB", &env.state);
    cmd.env("SAYA_PROVIDER", "openai_compatible");
    cmd.env("SAYA_MODEL", "mock-model");
    cmd.env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"));
    cmd.env("SAYA_API_KEY", "mock-secret");
    let mut child = pair.slave.spawn_command(cmd).expect("the child must spawn");
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().expect("the pty reader");
    let mut writer = pair.master.take_writer().expect("the pty writer");

    // Drain the terminal stream on a thread; the main loop watches for the
    // prompt and then for the exit.
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    thread::spawn(move || {
        let mut buffer = [0_u8; 4096];
        while let Ok(n) = reader.read(&mut buffer) {
            if n == 0 || tx.send(buffer[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    let mut stream = String::new();
    let started = std::time::Instant::now();
    let prompt_seen = loop {
        if started.elapsed() > DEADLINE {
            let _ = child.kill();
            panic!("the approval prompt never appeared; stream:\n{stream}");
        }
        if let Ok(bytes) = rx.recv_timeout(Duration::from_millis(100)) {
            stream.push_str(&String::from_utf8_lossy(&bytes));
        }
        if stream.contains("Approve this plan") {
            break true;
        }
    };
    let _ = prompt_seen;
    // The one answer: an explicit yes — nothing else approves.
    writer.write_all(answer.as_bytes()).expect("the pty writer");
    while child
        .try_wait()
        .expect("the child must be pollable")
        .is_none()
    {
        if started.elapsed() > DEADLINE {
            let _ = child.kill();
            panic!("the run never exited after the approval; stream:\n{stream}");
        }
        thread::sleep(Duration::from_millis(50));
        if let Ok(bytes) = rx.try_recv() {
            stream.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    let exit_code = child
        .try_wait()
        .expect("the child exited")
        .map(|status| status.exit_code() as i32);
    // Drain whatever remains (the run's stdout) for the completion asserts.
    while let Ok(bytes) = rx.try_recv() {
        stream.push_str(&String::from_utf8_lossy(&bytes));
    }
    InteractiveRun { exit_code, stream }
}

/// The journal of the one run in the scratch tree.
fn only_run_journal(env: &TestEnv) -> String {
    let mut entries = fs::read_dir(&env.runs)
        .expect("the runs root exists")
        .flatten()
        .map(|entry| entry.path())
        .collect::<Vec<PathBuf>>();
    assert_eq!(entries.len(), 1, "exactly one run exists: {entries:?}");
    fs::read_to_string(entries.remove(0).join("events.ndjson")).expect("the journal is readable")
}

/// Interactive approval of the plan satisfies it and the run proceeds: the
/// approval view is shown once, an explicit yes approves, and the run
/// completes — with no per-tool-call prompt afterwards.
#[test]
fn interactive_approval_of_the_plan_satisfies_it_and_the_run_proceeds() {
    let env = test_root("interactive-approval");
    let (address, _ready) = mock(vec![
        Scripted {
            body: plan_body(&["survey the data quality"]),
            delay_ms: 0,
        },
        Scripted {
            body: sse("survey complete"),
            delay_ms: 0,
        },
    ]);
    let run = run_interactively(
        &env,
        &[
            "run",
            "--allow",
            "workspace-write",
            "survey the data quality",
        ],
        &address,
        "y\n",
    );
    assert_eq!(
        run.exit_code,
        Some(0),
        "the approved run must complete; stream:\n{}",
        run.stream
    );
    // The approval view named what was being approved.
    assert!(
        run.stream.contains("survey the data quality"),
        "the view shows the plan: {}",
        run.stream
    );
    assert!(
        run.stream.contains("approved scopes: workspace-write"),
        "the view shows the granted scopes: {}",
        run.stream
    );
    // The one approval is on the durable record, and the run completed.
    let journal = only_run_journal(&env);
    assert_eq!(
        journal.matches("plan_approved").count(),
        1,
        "exactly one approval interaction: {journal}"
    );
    assert!(
        journal.contains("\"completed\""),
        "the approved run proceeded to completion: {journal}"
    );
    let _ = fs::remove_dir_all(&env.root);
}

/// A refusal at the approval gate refuses the run: exit 2, the run stays
/// unapproved, and nothing executed.
#[test]
fn refusing_the_plan_at_the_approval_gate_refuses_the_run() {
    let env = test_root("interactive-deny");
    let (address, _ready) = mock(vec![
        Scripted {
            body: plan_body(&["survey the data quality"]),
            delay_ms: 0,
        },
        Scripted {
            body: sse("never reached"),
            delay_ms: 0,
        },
    ]);
    let run = run_interactively(
        &env,
        &[
            "run",
            "--allow",
            "workspace-write",
            "survey the data quality",
        ],
        &address,
        "n\n",
    );
    assert_eq!(
        run.exit_code,
        Some(2),
        "a refused plan is a usage refusal; stream:\n{}",
        run.stream
    );
    assert!(
        run.stream.contains("was not approved"),
        "the refusal must say why: {}",
        run.stream
    );
    let journal = only_run_journal(&env);
    assert!(
        !journal.contains("\"completed\"") && !journal.contains("step_started"),
        "a refused run executed nothing: {journal}"
    );
    let _ = fs::remove_dir_all(&env.root);
}
