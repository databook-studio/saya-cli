//! The cancellation parity the TUI run panel depends on: the panel's
//! in-process stop (`record_cancelled`, the same function the panel worker
//! calls) and `saya run cancel` must write the *same durable record* — the
//! engine path, the journal tail, and the store status — never two
//! cancellation implementations that drift.

use super::{CancelOutcome, record_cancelled};
use crate::cli::RunCommand;
use crate::commands::run_management;
use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;
use saya_harness::journal::Journal;
use saya_harness::run_dir::RunDir;
use saya_store::{NewRun, RunBudgets, RunCapabilityFlags, RunStatus, RunStore, SqliteStateStore};
use saya_types::{RunEvent, RunId};
use std::{collections::BTreeMap, fs, path::Path, path::PathBuf};

pub(in crate::commands::run) fn temp_root(label: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-run-cancel-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

pub(in crate::commands::run) fn runtime_at(root: &Path) -> RuntimeConfig {
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
    let options = crate::GlobalOptions {
        connections: Some(connections),
        ..Default::default()
    };
    // The user dir is the test root too: no real user config is read, and
    // the resolved runtime is entirely this test's own.
    crate::config::runtime::load_with_sources(&options, root, root, BTreeMap::new()).unwrap()
}

/// Claims one approved-but-idle run the way the engine leaves a planned run
/// standing: directory, store row, journal through `PlanApproved`.
async fn seed_claimed_run(root: &Path, store: &SqliteStateStore, id: &str) -> RunId {
    let run_id = RunId::parse(id).unwrap();
    let run_dir = RunDir::create(&root.join("runs"), &run_id).unwrap();
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
    let journal = Journal::open(run_dir.root());
    for event in [
        RunEvent::RunStarted,
        RunEvent::PlanApproved { scopes: None },
    ] {
        journal.append(&event).unwrap();
    }
    RunStore::set_run_status(store, &run_id, RunStatus::Approved, None)
        .await
        .unwrap();
    run_id
}

/// The TUI panel's stop: the same function the worker calls when the panel's
/// token fires before the engine observed it. `wire` is `None` here — the
/// headless-shaped record; the panel's own wire observes the event through
/// its journal, which the run-panel tests pin.
async fn cancel_like_the_panel(root: &Path, store: &SqliteStateStore, run_id: &RunId) {
    let dir = root.join("runs").join(run_id.as_str());
    match record_cancelled(run_id, &dir, store, None).await {
        CancelOutcome::Recorded => {}
        other => panic!("the panel stop should record, got {other:?}"),
    }
}

/// The durable record of one run: the journal's tail event and the store's
/// status, the two things a second surface would drift on.
async fn durable_record(
    store: &SqliteStateStore,
    root: &Path,
    run_id: &RunId,
) -> (RunEvent, RunStatus) {
    let journal = Journal::open(root.join("runs").join(run_id.as_str()));
    let events = journal.read().unwrap();
    let status = RunStore::get_run(store, run_id)
        .await
        .unwrap()
        .unwrap()
        .status;
    (events.last().unwrap().clone(), status)
}

/// Cancelling the same way both surfaces do: the panel's stop and
/// `saya run cancel` on twin runs leave identical records — journal tail and
/// store status — and a later `saya run cancel` of the panel-cancelled run
/// reports it already finished, not a second cancellation.
#[tokio::test]
async fn the_panel_stop_and_saya_run_cancel_write_the_same_record() {
    let root = temp_root("parity");
    // `runs_dir()` resolves `SAYA_RUNS_DIR` at call time; this is the only
    // unit test in this crate that touches it, and it restores on exit.
    let previous = std::env::var_os("SAYA_RUNS_DIR");
    // SAFETY: single-threaded test-local mutation; `previous` is restored
    // before the function returns, and no other unit test reads the variable.
    unsafe { std::env::set_var("SAYA_RUNS_DIR", root.join("runs")) };
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let runtime = runtime_at(&root);
    let panel_run = seed_claimed_run(&root, &store, "r-panel-cancel").await;
    let headless_run = seed_claimed_run(&root, &store, "r-headless-cancel").await;

    cancel_like_the_panel(&root, &store, &panel_run).await;
    let code = run_management(
        RunCommand::Cancel {
            run_id: headless_run.as_str().to_string(),
        },
        &runtime,
        RenderFormat::Text,
        saya_agent::ApprovalPolicy::ReadOnly,
        &store,
    )
    .await
    .expect("the headless cancel succeeds");
    assert_eq!(code, 0, "cancelling a live-holder-free run succeeds");

    let (panel_tail, panel_status) = durable_record(&store, &root, &panel_run).await;
    let (headless_tail, headless_status) = durable_record(&store, &root, &headless_run).await;
    assert_eq!(
        panel_tail, headless_tail,
        "both surfaces leave the same journal tail"
    );
    assert_eq!(
        panel_tail,
        RunEvent::Cancelled,
        "the tail is the cancellation"
    );
    assert_eq!(panel_status, RunStatus::Cancelled);
    assert_eq!(
        panel_status, headless_status,
        "the store statuses cannot disagree"
    );

    // The panel-cancelled run reads as already finished to the CLI surface —
    // one cancellation, wherever it came from.
    let again = run_management(
        RunCommand::Cancel {
            run_id: panel_run.as_str().to_string(),
        },
        &runtime,
        RenderFormat::Text,
        saya_agent::ApprovalPolicy::ReadOnly,
        &store,
    )
    .await
    .unwrap();
    assert_eq!(again, 0);
    match previous {
        Some(value) => {
            // SAFETY: same single-threaded test scope as above.
            unsafe { std::env::set_var("SAYA_RUNS_DIR", value) };
        }
        None => {
            // SAFETY: same single-threaded test scope as above.
            unsafe { std::env::remove_var("SAYA_RUNS_DIR") };
        }
    }
    let _ = fs::remove_dir_all(root);
}
