//! The cancellation parity the TUI run panel depends on: the panel's
//! in-process stop (`record_cancelled`, the same function the panel worker
//! calls) and `saya run cancel` must write the *same durable record* — the
//! engine path, the journal tail, and the store status — never two
//! cancellation implementations that drift.

use super::{CancelOutcome, record_cancelled};
use crate::cli::RunCommand;
use crate::commands::{
    run::{HostRun, RunRequest, start_for_panel},
    run_management,
};
use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;
use saya_harness::journal::Journal;
use saya_harness::lock::RunLock;
use saya_harness::run_dir::RunDir;
use saya_store::{NewRun, RunBudgets, RunCapabilityFlags, RunStatus, RunStore, SqliteStateStore};
use saya_types::{Capabilities, RunEvent, RunId, RunPlan, StepSpec};
use std::{collections::BTreeMap, ffi::OsString, fs, path::Path, path::PathBuf, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{Mutex, oneshot},
};

static RUNS_DIR_TEST_LOCK: Mutex<()> = Mutex::const_new(());

struct RunsDirEnv(Option<OsString>);

impl RunsDirEnv {
    fn set(path: PathBuf) -> Self {
        let previous = std::env::var_os("SAYA_RUNS_DIR");
        // SAFETY: both tests that mutate this variable hold the shared test
        // mutex and restore it through this guard before returning.
        unsafe { std::env::set_var("SAYA_RUNS_DIR", path) };
        Self(previous)
    }
}

impl Drop for RunsDirEnv {
    fn drop(&mut self) {
        match self.0.take() {
            // SAFETY: the two test callers retain RUNS_DIR_TEST_LOCK through
            // this guard's drop, serializing process-environment mutation.
            Some(value) => unsafe { std::env::set_var("SAYA_RUNS_DIR", value) },
            // SAFETY: same serialized test scope as the restore arm above.
            None => unsafe { std::env::remove_var("SAYA_RUNS_DIR") },
        }
    }
}

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

include!("cancel_tests/durable_record.rs");

include!("cancel_tests/lock_lifecycle.rs");

include!("cancel_tests/wall_clock_exit.rs");

async fn read_http_request(socket: &mut tokio::net::TcpStream) -> Vec<u8> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 2048];
    loop {
        let count = socket.read(&mut buffer).await.unwrap();
        assert_ne!(count, 0, "provider request closes before headers complete");
        request.extend_from_slice(&buffer[..count]);
        let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&request[..end]);
        let length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length: ")
                    .and_then(|value| value.parse::<usize>().ok())
            })
            .expect("Ollama request has a content length");
        if request.len() >= end + 4 + length {
            return request;
        }
    }
}

async fn write_ollama_response(socket: &mut tokio::net::TcpStream, content: &str) {
    let body = format!(
        "{}\n",
        serde_json::json!({"message": {"content": content}, "done": true})
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    socket.write_all(response.as_bytes()).await.unwrap();
}
