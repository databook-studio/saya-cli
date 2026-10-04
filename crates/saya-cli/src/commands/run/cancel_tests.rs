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

/// Cancelling the same way both surfaces do: the panel's stop and
/// `saya run cancel` on twin runs leave identical records — journal tail and
/// store status — and a later `saya run cancel` of the panel-cancelled run
/// reports it already finished, not a second cancellation.
#[tokio::test]
async fn the_panel_stop_and_saya_run_cancel_write_the_same_record() {
    let _env_lock = RUNS_DIR_TEST_LOCK.lock().await;
    let root = temp_root("parity");
    let _runs_dir = RunsDirEnv::set(root.join("runs"));
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
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn panel_start_keeps_run_lock_until_cancelled_plan_source_returns() {
    let _env_lock = RUNS_DIR_TEST_LOCK.lock().await;
    let root = temp_root("start-settlement");
    let _runs_dir = RunsDirEnv::set(root.join("runs"));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (request_tx, request_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = read_http_request(&mut socket).await;
        let _ = request_tx.send(request);
        release_rx.await.unwrap();
        let body = "source failed after the panel stop";
        let response = format!(
            "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    });

    let mut runtime = runtime_at(&root);
    let endpoint = runtime
        .resolved
        .endpoints
        .get_mut(saya_config::ORCHESTRATOR_ROLE)
        .unwrap();
    endpoint.provider = saya_config::AiProvider::Ollama;
    endpoint.base_url = Some(format!("http://{address}"));
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let run_id = RunId::parse("r-start-settlement").unwrap();
    let cancellation = saya_agent::CancellationToken::default();
    let approval = super::super::approval::PlanApproval::PreAuthorized;
    let active = start_for_panel(
        RunRequest {
            run_id: run_id.clone(),
            goal: Some("observe safely".into()),
            allow: vec!["none".into()],
            budget: Vec::new(),
        },
        &runtime,
        RenderFormat::Text,
        saya_agent::ApprovalPolicy::ReadOnly,
        &store,
        HostRun {
            journal_wire: None,
            agent_stream: None,
            profile: None,
            plan_approval: &approval,
            cancellation: cancellation.clone(),
        },
    );
    tokio::pin!(active);
    let request = tokio::select! {
        biased;
        result = &mut active => panic!("panel start returned before the provider request: {result:?}"),
        request = request_rx => request.unwrap(),
    };
    assert!(String::from_utf8_lossy(&request).contains("/api/chat"));

    cancellation.cancel();
    tokio::select! {
        biased;
        result = &mut active => panic!("panel start returned before source settlement: {result:?}"),
        _ = tokio::task::yield_now() => {}
    }
    let lock = root.join("runs").join(run_id.as_str()).join("lock");
    assert!(
        RunLock::acquire(&lock).is_err(),
        "the started run keeps its single-writer lock until its plan request returns"
    );

    release_tx.send(()).unwrap();
    let code = tokio::time::timeout(Duration::from_secs(2), &mut active)
        .await
        .expect("panel drive settles after local provider response")
        .unwrap();
    assert_eq!(code, 130);
    server.await.unwrap();
    let events = Journal::open(root.join("runs").join(run_id.as_str()))
        .read()
        .unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RunEvent::Cancelled))
            .count(),
        1
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, RunEvent::StepStarted { .. }))
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn panel_start_maps_settled_wall_clock_stop_to_exit_six() {
    let _env_lock = RUNS_DIR_TEST_LOCK.lock().await;
    let root = temp_root("start-deadline");
    let _runs_dir = RunsDirEnv::set(root.join("runs"));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (episode_tx, episode_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let plan_content = serde_json::to_string(
        &RunPlan::new(vec![
            StepSpec::new(
                "wait for answer",
                Capabilities::default(),
                None,
                Vec::new(),
                None,
            )
            .unwrap(),
        ])
        .unwrap(),
    )
    .unwrap();
    let server = tokio::spawn(async move {
        let (mut planning, _) = listener.accept().await.unwrap();
        let _ = read_http_request(&mut planning).await;
        write_ollama_response(&mut planning, &plan_content).await;

        let (mut episode, _) = listener.accept().await.unwrap();
        let _ = read_http_request(&mut episode).await;
        let _ = episode_tx.send(());
        release_rx.await.unwrap();
    });

    let mut runtime = runtime_at(&root);
    let endpoint = runtime
        .resolved
        .endpoints
        .get_mut(saya_config::ORCHESTRATOR_ROLE)
        .unwrap();
    endpoint.provider = saya_config::AiProvider::Ollama;
    endpoint.base_url = Some(format!("http://{address}"));
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let run_id = RunId::parse("r-start-deadline").unwrap();
    let cancellation = saya_agent::CancellationToken::default();
    let approval = super::super::approval::PlanApproval::PreAuthorized;
    let active = start_for_panel(
        RunRequest {
            run_id: run_id.clone(),
            goal: Some("wait safely".into()),
            allow: vec!["none".into()],
            budget: vec!["wall-clock=1".into()],
        },
        &runtime,
        RenderFormat::Text,
        saya_agent::ApprovalPolicy::ReadOnly,
        &store,
        HostRun {
            journal_wire: None,
            agent_stream: None,
            profile: None,
            plan_approval: &approval,
            cancellation: cancellation.clone(),
        },
    );
    tokio::pin!(active);
    tokio::select! {
        biased;
        result = &mut active => panic!("panel start ended before the episode source began: {result:?}"),
        started = episode_rx => started.unwrap(),
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        tokio::select! {
            biased;
            result = &mut active => panic!("panel start ended before its stop token: {result:?}"),
            _ = cancellation.cancelled() => {},
        }
    })
    .await
    .expect("independent run deadline signals the panel's shared token");

    let code = tokio::time::timeout(Duration::from_secs(2), &mut active)
        .await
        .expect("the local provider attempt settles")
        .unwrap();
    assert_eq!(code, 6);
    let events = Journal::open(root.join("runs").join(run_id.as_str()))
        .read()
        .unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RunEvent::Paused { .. }))
            .count(),
        1
    );
    assert!(events.contains(&RunEvent::Paused {
        reason: saya_types::PauseReason::WallClockExceeded,
    }));

    release_tx.send(()).unwrap();
    server.await.unwrap();
    let _ = fs::remove_dir_all(root);
}

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
