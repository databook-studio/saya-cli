//! Cross-adapter parity for the `/runs` family: the `/runs` and `/run cancel`
//! slash commands must call the *same* read path as the headless
//! `saya run list|show|cancel` commands and add nothing — one dispatcher
//! (`run_management`), one renderer (`crate::render_run`), so the two
//! adapters cannot drift into two renderers for the same run.
//!
//! Detection, not demonstration: each test runs the slash path and the
//! headless path against the same store + runs root and asserts they agree on
//! the thing that would diverge if a second code path had snuck in — the
//! rendered bytes. `/runs <id>` and `saya run show <id>` must be
//! byte-identical; an unknown id must fail identically on both.
//!
//! Recipe: `contracts_slash_parity.rs` (same harness shape, same capture
//! seam, same parity assertions on captured stdout/stderr).

use saya_cli::{
    RenderFormat, RunCommand, RuntimeConfig, SlashCommand, capture_output_start,
    capture_output_take, load_with_sources, parse_slash_command, run_management,
};
use saya_harness::journal::Journal;
use saya_store::{NewRun, RunBudgets, RunCapabilityFlags, RunStatus, RunStore, SqliteStateStore};
use saya_types::{Budgets, Capabilities, RunEvent, RunId, RunSpec};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

// ---------------------------------------------------------------------------
// harness — mirrors tests/contracts_slash_parity.rs so the two suites share
// one store shape.
// ---------------------------------------------------------------------------

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-run-slash-parity-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn runtime_at(root: &Path) -> RuntimeConfig {
    let database = root.join("data.sqlite3");
    fs::write(&database, b"").unwrap();
    let connections = root.join("connections.toml");
    fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n",
            database.display()
        ),
    )
    .unwrap();
    let options = saya_cli::GlobalOptions {
        connections: Some(connections),
        ..Default::default()
    };
    load_with_sources(&options, root, root, BTreeMap::new()).unwrap()
}

async fn store_at(root: &Path) -> SqliteStateStore {
    SqliteStateStore::new(root.join("state.sqlite3"))
}

/// The env lock: `SAYA_RUNS_DIR` is process-global, and the tests in this
/// binary run concurrently. Every env-touching test takes this async lock for
/// its whole body — including across awaits — so another test in this file
/// never observes a torn or foreign runs root. Tokio's own `Mutex` because a
/// std guard held across an await point is exactly the deadlock clippy names.
static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn lock_env() -> tokio::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().await
}

/// SAFETY: `set_var` mutates process-global state. The caller holds
/// `ENV_LOCK` for the whole test, so no other test in this binary observes a
/// torn value; every other integration test is a separate process, and each
/// test points the variable at its own private root and restores it on exit.
unsafe fn set_runs_dir(path: &Path) -> Option<std::ffi::OsString> {
    let previous = std::env::var_os("SAYA_RUNS_DIR");
    // SAFETY: the caller holds `ENV_LOCK` for the whole test body, so no other
    // test in this binary observes a torn or foreign runs root.
    unsafe { std::env::set_var("SAYA_RUNS_DIR", path) };
    previous
}

/// SAFETY: see [`set_runs_dir`]; the caller still holds `ENV_LOCK`.
unsafe fn restore_runs_dir(previous: Option<std::ffi::OsString>) {
    if let Some(value) = previous {
        // SAFETY: same lock discipline as `set_runs_dir`.
        unsafe { std::env::set_var("SAYA_RUNS_DIR", value) };
    } else {
        // SAFETY: same lock discipline as `set_runs_dir`.
        unsafe { std::env::remove_var("SAYA_RUNS_DIR") };
    }
}

/// Seeds one completed run the way the engine leaves one: a run directory
/// with its spec and journal, plus the store's metadata row, written through
/// the store and journal the engine itself uses. The directory the read
/// surfaces resolve (`runs_dir()`) must be `root/runs`, so the caller holds
/// the env lock.
async fn seed_run(root: &Path, store: &SqliteStateStore, id: &str, goal: &str, usage: &[RunEvent]) {
    let run_dir = root.join("runs").join(id);
    fs::create_dir_all(&run_dir).unwrap();
    let run_id = RunId::parse(id).unwrap();
    // `Capabilities` is #[non_exhaustive]: assemble through the default and
    // flip the approved scope, the same way the CLI flags do.
    let mut scopes = Capabilities::default();
    scopes.workspace_write = true;
    let spec = RunSpec::new(run_id.clone(), goal, scopes, Budgets::default()).unwrap();
    fs::write(
        run_dir.join("spec.json"),
        serde_json::to_string(&spec).unwrap(),
    )
    .unwrap();
    RunStore::create_run(
        store,
        NewRun {
            id: run_id.clone(),
            capabilities: RunCapabilityFlags {
                workspace_write: true,
                ..Default::default()
            },
            budgets: RunBudgets::default(),
        },
    )
    .await
    .unwrap();
    let journal = Journal::open(&run_dir);
    // Usage rides where the engine journals it: per call, before the run's
    // completion — the durable record the show stanza folds.
    for event in [
        RunEvent::RunStarted,
        RunEvent::PlanApproved { scopes: None },
        RunEvent::StepStarted { step: 0 },
        RunEvent::StepCompleted { step: 0 },
    ] {
        journal.append(&event).unwrap();
    }
    for event in usage {
        journal.append(event).unwrap();
    }
    journal.append(&RunEvent::Completed).unwrap();
    // The store's status machine is walked the legal way a real run walks it
    // (planned → approved → executing → completed); an illegal transition is
    // refused, which is the store being the mirror, not the authority.
    for status in [
        RunStatus::Approved,
        RunStatus::Executing,
        RunStatus::Completed,
    ] {
        RunStore::set_run_status(store, &run_id, status, None)
            .await
            .unwrap();
    }
}

/// Runs a `RunCommand` through the shared dispatcher, capturing the exact
/// bytes a headless command would print.
async fn run_headless(
    command: RunCommand,
    runtime: &RuntimeConfig,
    store: &SqliteStateStore,
    format: RenderFormat,
) -> (i32, String, String) {
    capture_output_start();
    let code = run_management(
        command,
        runtime,
        format,
        saya_agent::ApprovalPolicy::ReadOnly,
        store,
    )
    .await
    .unwrap();
    let (out, err) = capture_output_take();
    (code, out, err)
}

/// The slash side: parse the line and route it the way the session loop does
/// (`/runs <id>` → `Show`), then the same dispatcher. Returns the parsed
/// command so the translation itself is under test.
fn parse_runs(line: &str) -> Option<String> {
    match parse_slash_command(line) {
        Ok(Some(SlashCommand::Runs(run_id))) => run_id,
        other => panic!("expected SlashCommand::Runs for {line:?}, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 1. `/runs <id>` and `saya run show <id>` are byte-identical for the same
//    run; `/runs` and `saya run list` likewise.
// ---------------------------------------------------------------------------
include!("run_slash_parity/list_parity.rs");

// ---------------------------------------------------------------------------
// 3. The JSON framing is one serde path: `/runs <id>` in JSON renders the
//    same result envelope as the headless command, and a run event's NDJSON
//    line is exactly the journal's line.
// ---------------------------------------------------------------------------
include!("run_slash_parity/rendering_parity.rs");
