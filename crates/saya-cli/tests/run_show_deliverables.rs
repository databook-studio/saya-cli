//! The deliverables half of `saya run show`: a step's declared outputs
//! (`StepSpec.expects`) become an artifact manifest at completion, and
//! `saya run show <id>` renders it — name, size, digest. Five guarantees,
//! driven through the real binary against a scripted local provider
//! (`run_cli.rs` and `tests/mvp/provider.rs` are the recipe):
//!
//! 1. A completed run's deliverables are listed with sizes and digests.
//! 2. A declared deliverable the step never produced is reported missing,
//!    distinctly from one that exists.
//! 3. A deliverable naming a path outside the workspace is refused — the
//!    outside sentinel is neither read nor listed.
//! 4. Two runs' deliverables never bleed: `show` on run A never lists run
//!    B's files.
//! 5. The recorded digest is the content's at completion; corrupting the
//!    file afterwards changes nothing — the record is what completed.

use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    process::{Command as ProcessCommand, Output},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

/// The exact content a scripted step writes, and the sha256 `shasum -a 256`
/// pins for it — the digest `run show` must record.
const REPORT_CONTENT: &str = "saya deliverable report body\n";
const REPORT_DIGEST: &str = "46348b7d9d3ca11fa3fd2fedd51d45b0b55221b58edb9cae9e47c2f54bdd1f34";
const A_CONTENT: &str = "run a artifact\n";
const A_DIGEST: &str = "32f4203dca774f15271920fb89dc67b06a5b9f2ff7d3f4cc2c0b40c8f199221a";
const B_CONTENT: &str = "run b artifact\n";
const B_DIGEST: &str = "875ce39e11fba5c61ce64d6c61928629840e9b8e493482d2e9672e930b9b7ab8";
const OUTSIDE_CONTENT: &str = "outside sentinel\n";

/// One scripted provider response: its SSE body and how long to wait before
/// sending it.
struct Scripted {
    body: String,
    delay_ms: u64,
}

/// A scripted local provider: one response per connection, in order; the
/// script spent, every further connection is accepted and dropped — a failed
/// call, never a hang. The thread never terminates on its own; tests drop
/// the handle and it dies with the test process (`run_cli.rs` is the
/// pattern). The flag flips once the first connection has been read.
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
                    thread::sleep(Duration::from_millis(scripted.delay_ms));
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

/// One streaming response carrying `content` as the message's text — no tool
/// calls, so the loop terminates.
fn sse(content: &str) -> String {
    let chunk = serde_json::json!({"choices": [{"delta": {"content": content}}]});
    format!("data: {chunk}\n\ndata: [DONE]\n\n")
}

/// The planner's answer: one step with `workspace_write` and the declared
/// expected outputs — the shape `plan/prompt.rs` demands.
fn plan_with_expects(expects: &[(&str, &str)]) -> String {
    let step = serde_json::json!({
        "goal": "produce the declared artifacts",
        "capabilities": {"workspace_write": true},
        "budget": serde_json::Value::Null,
        "expects": expects
            .iter()
            .map(|(name, description)| {
                serde_json::json!({"name": name, "description": description})
            })
            .collect::<Vec<_>>(),
        "endpoint": serde_json::Value::Null
    });
    sse(&serde_json::json!({"steps": [step]}).to_string())
}

/// One streaming response carrying a single `workspace_write` tool call.
fn write_call(path: &str, content: &str) -> String {
    let chunk = serde_json::json!({
        "choices": [{"delta": {"tool_calls": [{
            "index": 0,
            "id": "call_write",
            "function": {
                "name": "workspace_write",
                "arguments": serde_json::json!({"path": path, "content": content}).to_string()
            }
        }]}}]
    });
    format!("data: {chunk}\n\ndata: [DONE]\n\n")
}

struct TestEnv {
    root: PathBuf,
    runs: PathBuf,
    state: PathBuf,
}

/// A per-test scratch root: config home, runs root, and state database all
/// live under one tree.
fn test_root(label: &str) -> TestEnv {
    let root = std::env::temp_dir().join(format!(
        "saya-run-show-deliverables-{label}-{}",
        std::process::id()
    ));
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

/// Every run id `saya run list` shows, in listing order.
fn listed_ids(env: &TestEnv, address: &str) -> Vec<String> {
    let listing = saya(env, &["run", "list"], address);
    assert_eq!(
        listing.status.code(),
        Some(0),
        "stderr: {}",
        stderr(&listing)
    );
    stdout(&listing)
        .lines()
        .filter_map(|line| line.split('\t').next())
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect()
}

/// A run's goal, read from its spec file — how a test tells two run ids
/// apart.
fn goal_of(env: &TestEnv, id: &str) -> String {
    let spec = fs::read_to_string(env.runs.join(id).join("spec.json")).unwrap();
    serde_json::from_str::<serde_json::Value>(&spec).unwrap()["goal"]
        .as_str()
        .unwrap()
        .to_string()
}

/// The one-run convenience: drives the scripted run to completion and
/// returns its id — the id that appears in the listing and was not there
/// before the run started.
fn completed_run(env: &TestEnv, script: Vec<Scripted>, goal: &str) -> String {
    let (address, ready) = mock(script);
    let before = listed_ids(env, &address);
    let started = saya(
        env,
        &[
            "--non-interactive",
            "run",
            "--allow",
            "workspace-write",
            goal,
        ],
        &address,
    );
    assert!(
        ready.load(Ordering::SeqCst),
        "the binary never reached its provider call"
    );
    assert_eq!(
        started.status.code(),
        Some(0),
        "the run must complete; stderr: {}",
        stderr(&started)
    );
    let after = listed_ids(env, &address);
    let new: Vec<&String> = after.iter().filter(|id| !before.contains(id)).collect();
    assert_eq!(new.len(), 1, "exactly one new run must exist");
    new.into_iter().next().unwrap().clone()
}

/// A completed one-step run that declares `expects` and writes `path` with
/// `content` through the workspace tool.
fn run_writing(
    env: &TestEnv,
    expects: &[(&str, &str)],
    path: &str,
    content: &str,
    goal: &str,
) -> String {
    completed_run(
        env,
        vec![
            Scripted {
                body: plan_with_expects(expects),
                delay_ms: 0,
            },
            Scripted {
                body: write_call(path, content),
                delay_ms: 0,
            },
            Scripted {
                body: sse("step complete"),
                delay_ms: 0,
            },
        ],
        goal,
    )
}

/// A completed one-step run that declares `expects` and writes nothing.
fn run_writing_nothing(env: &TestEnv, expects: &[(&str, &str)], goal: &str) -> String {
    completed_run(
        env,
        vec![
            Scripted {
                body: plan_with_expects(expects),
                delay_ms: 0,
            },
            Scripted {
                body: sse("no artifact written"),
                delay_ms: 0,
            },
        ],
        goal,
    )
}

fn show(env: &TestEnv, id: &str, address: &str) -> Output {
    saya(env, &["run", "show", id], address)
}

/// A completed run's declared deliverables are listed with their size and
/// digest.
#[test]
fn a_completed_run_shows_its_deliverables_with_sizes_and_digests() {
    let env = test_root("delivered");
    let id = run_writing(
        &env,
        &[("report.md", "the survey report")],
        "report.md",
        REPORT_CONTENT,
        "produce the survey report",
    );
    let (address, _ready) = mock(Vec::new());
    let show = show(&env, &id, &address);
    assert_eq!(show.status.code(), Some(0), "stderr: {}", stderr(&show));
    let shown = stdout(&show);
    assert!(
        shown.contains("report.md"),
        "the deliverable's name must be listed: {shown}"
    );
    assert!(
        shown.contains(&format!("{} bytes", REPORT_CONTENT.len())),
        "the deliverable's size must be listed: {shown}"
    );
    assert!(
        shown.contains(REPORT_DIGEST),
        "the deliverable's digest must be listed: {shown}"
    );
    // The manifest is durable: the journal carries it, digest included.
    let journal = fs::read_to_string(env.runs.join(&id).join("events.ndjson")).unwrap();
    assert!(
        journal.contains("deliverables") && journal.contains(REPORT_DIGEST),
        "the journal must record the manifest: {journal}"
    );
    let _ = fs::remove_dir_all(&env.root);
}

/// A declared deliverable the step never produced is reported missing,
/// distinctly from one that exists — a run may not look complete while its
/// declared outputs are absent.
#[test]
fn a_declared_deliverable_the_step_never_produced_is_reported_missing() {
    let env = test_root("missing");
    let id = run_writing_nothing(
        &env,
        &[("report.md", "the survey report")],
        "produce the survey report",
    );
    let (address, _ready) = mock(Vec::new());
    let show = show(&env, &id, &address);
    assert_eq!(show.status.code(), Some(0), "stderr: {}", stderr(&show));
    let shown = stdout(&show);
    assert!(
        shown.contains("report.md missing"),
        "the declared-but-absent deliverable must be reported missing: {shown}"
    );
    assert!(
        !shown.contains("sha256"),
        "a missing deliverable must not carry a digest: {shown}"
    );
    let _ = fs::remove_dir_all(&env.root);
}

/// A deliverable naming a path outside the workspace is refused: the plan
/// never binds, the run stops loudly, and the outside sentinel is neither
/// read nor listed anywhere.
#[test]
fn a_deliverable_naming_a_path_outside_the_workspace_is_refused() {
    let env = test_root("outside");
    let sentinel = env.root.join("sentinel.txt");
    fs::write(&sentinel, OUTSIDE_CONTENT).unwrap();
    // The model proposes the same escaping plan on every attempt; the bound
    // is spent and the run stops without binding it.
    let escaping_plan = plan_with_expects(&[("../sentinel.txt", "the outside file")]);
    let (address, _ready) = mock(vec![
        Scripted {
            body: escaping_plan.clone(),
            delay_ms: 0,
        },
        Scripted {
            body: escaping_plan.clone(),
            delay_ms: 0,
        },
        Scripted {
            body: escaping_plan,
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
            "produce the outside file",
        ],
        &address,
    );
    assert_eq!(
        started.status.code(),
        Some(5),
        "an escaping deliverable must refuse the plan; stderr: {}",
        stderr(&started)
    );
    assert!(
        stderr(&started).contains("could not bind a plan"),
        "the refusal must say the plan never bound: {}",
        stderr(&started)
    );
    assert!(
        stderr(&started).contains("expected output"),
        "the refusal must name the offending expectation: {}",
        stderr(&started)
    );
    // The sentinel is intact — never read for a digest, never written.
    assert_eq!(
        fs::read_to_string(&sentinel).unwrap(),
        OUTSIDE_CONTENT,
        "the outside sentinel must be untouched"
    );
    // And it is listed nowhere: the run's own show has no deliverables.
    let ids = listed_ids(&env, &address);
    assert_eq!(ids.len(), 1, "exactly the refused run must exist");
    let shown = stdout(&show(&env, &ids[0], &address));
    assert!(
        !shown.contains("sentinel"),
        "the outside path must never be listed: {shown}"
    );
    let _ = fs::remove_dir_all(&env.root);
}

/// Two runs' deliverables do not bleed: `show` on run A never lists run B's
/// files, and the other way round.
#[test]
fn two_runs_deliverables_do_not_bleed() {
    let env = test_root("no-bleed");
    let id_b = run_writing(
        &env,
        &[("b-notes.md", "run b's notes")],
        "b-notes.md",
        B_CONTENT,
        "run b produces its notes",
    );
    let id_a = run_writing(
        &env,
        &[("a-report.md", "run a's report")],
        "a-report.md",
        A_CONTENT,
        "run a produces its report",
    );
    assert_ne!(id_a, id_b, "the two runs must be distinct");
    let (address, _ready) = mock(Vec::new());
    let (id_a, id_b) = if goal_of(&env, &id_a).contains("run a") {
        (id_a, id_b)
    } else {
        (id_b, id_a)
    };
    let shown_a = stdout(&show(&env, &id_a, &address));
    assert!(
        shown_a.contains("a-report.md") && shown_a.contains(A_DIGEST),
        "run A's deliverable must be listed: {shown_a}"
    );
    assert!(
        !shown_a.contains("b-notes.md") && !shown_a.contains(B_DIGEST),
        "run B's files must never appear in run A's show: {shown_a}"
    );
    let shown_b = stdout(&show(&env, &id_b, &address));
    assert!(
        shown_b.contains("b-notes.md") && shown_b.contains(B_DIGEST),
        "run B's deliverable must be listed: {shown_b}"
    );
    assert!(
        !shown_b.contains("a-report.md") && !shown_b.contains(A_DIGEST),
        "run A's files must never appear in run B's show: {shown_b}"
    );
    let _ = fs::remove_dir_all(&env.root);
}

/// The recorded digest matches the file's content at completion; corrupting
/// the file afterwards changes nothing — the record is what completed, not
/// what is there now.
#[test]
fn the_recorded_digest_is_what_completed_and_survives_later_corruption() {
    let env = test_root("digest");
    let id = run_writing(
        &env,
        &[("report.md", "the survey report")],
        "report.md",
        REPORT_CONTENT,
        "produce the survey report",
    );
    let (address, _ready) = mock(Vec::new());
    let shown = stdout(&show(&env, &id, &address));
    assert!(
        shown.contains(REPORT_DIGEST),
        "the recorded digest must match the completed content: {shown}"
    );
    // Corrupt the workspace file after completion.
    let artifact = env.runs.join(&id).join("workspace").join("report.md");
    fs::write(&artifact, "tampered after completion\n").unwrap();
    let shown_after = stdout(&show(&env, &id, &address));
    assert!(
        shown_after.contains(REPORT_DIGEST),
        "the recorded digest must be unchanged by later corruption: {shown_after}"
    );
    assert!(
        shown_after.contains(&format!("{} bytes", REPORT_CONTENT.len())),
        "the recorded size must be unchanged by later corruption: {shown_after}"
    );
    let _ = fs::remove_dir_all(&env.root);
}
