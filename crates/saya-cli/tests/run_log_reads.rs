//! `saya run log` reads the run journal back: every event, in write order,
//! one line each. Three guarantees:
//!
//! 1. Text mode renders the shaper's own line per event (`crate::render_run`
//!    is the one shaper) — write order, one line per recorded event.
//! 2. JSON and NDJSON keep the journal's own bytes — the one serde path the
//!    live wire streams — so a script parsing `--format ndjson` parses the
//!    durable record, not a re-rendering of it.
//! 3. A run whose journal holds no events says so, rather than succeeding
//!    with empty output that reads as "nothing happened is nothing".
//!
//! Recipe: `run_slash_parity.rs` (same store + runs-root harness).

use saya_cli::{
    RenderFormat, RunCommand, RuntimeConfig, capture_output_start, capture_output_take,
    load_with_sources, run_management,
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

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-run-log-{label}-{}-{stamp}",
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

static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn lock_env() -> tokio::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().await
}

/// SAFETY: see `run_slash_parity.rs`; the caller holds `ENV_LOCK` for the
/// whole test body.
unsafe fn set_runs_dir(path: &Path) -> Option<std::ffi::OsString> {
    let previous = std::env::var_os("SAYA_RUNS_DIR");
    // SAFETY: the caller holds `ENV_LOCK` for the whole test body.
    unsafe { std::env::set_var("SAYA_RUNS_DIR", path) };
    previous
}

/// SAFETY: see `set_runs_dir`; the caller still holds `ENV_LOCK`.
unsafe fn restore_runs_dir(previous: Option<std::ffi::OsString>) {
    match previous {
        Some(value) => {
            // SAFETY: same lock discipline as `set_runs_dir`.
            unsafe { std::env::set_var("SAYA_RUNS_DIR", value) };
        }
        None => {
            // SAFETY: same lock discipline as `set_runs_dir`.
            unsafe { std::env::remove_var("SAYA_RUNS_DIR") };
        }
    }
}

/// Seeds one run the way the engine leaves one, with exactly the journal
/// events given — the order the read surfaces must render back.
async fn seed_run(root: &Path, store: &SqliteStateStore, id: &str, events: &[RunEvent]) {
    let run_dir = root.join("runs").join(id);
    fs::create_dir_all(&run_dir).unwrap();
    let run_id = RunId::parse(id).unwrap();
    let mut scopes = Capabilities::default();
    scopes.workspace_write = true;
    let spec = RunSpec::new(
        run_id.clone(),
        "the seeded run's goal",
        scopes,
        Budgets::default(),
    )
    .unwrap();
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
    for event in events {
        journal.append(event).unwrap();
    }
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

async fn run_log(
    id: &str,
    runtime: &RuntimeConfig,
    store: &SqliteStateStore,
    format: RenderFormat,
) -> (i32, String, String) {
    capture_output_start();
    let code = run_management(
        RunCommand::Log {
            run_id: id.to_string(),
        },
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

/// The journal a typical completed one-step run with one provider call
/// leaves: lifecycle, the call's usage, and the completion — write order is
/// what `run log` must read back in.
fn seeded_events() -> Vec<RunEvent> {
    vec![
        RunEvent::RunStarted,
        RunEvent::PlanApproved { scopes: None },
        RunEvent::StepStarted { step: 0 },
        RunEvent::Usage {
            endpoint: "primary".into(),
            tokens: Some(100),
            turns: None,
            tool_calls: None,
            cached_input_tokens: None,
            cache_creation_input_tokens: None,
        },
        RunEvent::StepCompleted { step: 0 },
        RunEvent::Completed,
    ]
}

/// Text mode renders the shaper's own line per event, in write order, one
/// line per recorded event — including the usage event the engine journals
/// per provider call.
#[tokio::test]
async fn run_log_text_renders_the_journals_events_in_write_order() {
    let _env = lock_env().await;
    let root = temp_root("text");
    unsafe { set_runs_dir(&root.join("runs")) };
    let runtime = runtime_at(&root);
    let store = store_at(&root).await;
    let events = seeded_events();
    seed_run(&root, &store, "r-log-text", &events).await;

    let (code, out, err) = run_log("r-log-text", &runtime, &store, RenderFormat::Text).await;

    assert_eq!(code, 0, "stderr: {err}");
    let expected = [
        "run started",
        "plan approved",
        "step 1 started",
        "usage · primary · tokens 100 · cache reads unknown · cache writes unknown · turns unknown · tool calls unknown",
        "step 1 completed",
        "run completed",
    ]
    .join("\n");
    assert_eq!(
        out,
        format!("{expected}\n"),
        "the log is the journal, in write order, one line per event"
    );

    unsafe { restore_runs_dir(None) };
    let _ = fs::remove_dir_all(root);
}

/// JSON and NDJSON keep the journal's own bytes: the message carries the
/// events' one serde path, in write order — the framing machine consumers parse.
#[tokio::test]
async fn run_log_ndjson_carries_the_journals_own_lines() {
    let _env = lock_env().await;
    let root = temp_root("ndjson");
    unsafe { set_runs_dir(&root.join("runs")) };
    let runtime = runtime_at(&root);
    let store = store_at(&root).await;
    let events = seeded_events();
    seed_run(&root, &store, "r-log-wire", &events).await;

    let (code, out, err) = run_log("r-log-wire", &runtime, &store, RenderFormat::Ndjson).await;

    assert_eq!(code, 0, "stderr: {err}");
    let envelope: serde_json::Value = serde_json::from_str(out.trim_end()).unwrap();
    let message = envelope["message"].as_str().expect("result envelope");
    let expected = events
        .iter()
        .map(|event| serde_json::to_string(event).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        message, expected,
        "the NDJSON framing is the journal's own serialization, in write order"
    );

    unsafe { restore_runs_dir(None) };
    let _ = fs::remove_dir_all(root);
}

/// A run with no journal events renders something honest rather than an
/// empty success that reads as "the journal was empty on purpose".
#[tokio::test]
async fn a_run_with_no_journal_events_renders_something_honest() {
    let _env = lock_env().await;
    let root = temp_root("empty");
    unsafe { set_runs_dir(&root.join("runs")) };
    let runtime = runtime_at(&root);
    let store = store_at(&root).await;
    seed_run(&root, &store, "r-log-empty", &[]).await;

    let (code, out, err) = run_log("r-log-empty", &runtime, &store, RenderFormat::Text).await;

    assert_eq!(code, 0, "stderr: {err}");
    assert!(
        out.contains("no journal events"),
        "the honest message, not empty output: {out:?}"
    );

    unsafe { restore_runs_dir(None) };
    let _ = fs::remove_dir_all(root);
}
