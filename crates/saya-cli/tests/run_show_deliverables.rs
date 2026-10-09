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

include!("run_show_deliverables/provider.rs");
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

include!("run_show_deliverables/present_missing.rs");

include!("run_show_deliverables/containment.rs");

include!("run_show_deliverables/run_isolation.rs");
