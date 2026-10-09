//! Tests for the background saved-investigation replay (D13): `/investigation
//! run` runs on a worker thread the way a direct-SQL task does, the loop keeps
//! ticking while it runs, Esc/Ctrl+C detach it exactly like a SQL task, and a
//! completion — only when not detached — pushes the shared operation's
//! output, captures the typed result, and sets the latest selectable query.

use super::super::application::SecondSqlDecision;
use super::super::dispatch::Dispatch;
use super::super::dispatch_investigation::run_investigation;
use super::super::keys::handle_key;
use super::super::sql_task::{Followup, SqlTask};
use super::super::transcript::{BlockKind, Transcript};
use super::super::types::{App, LastQuery};
use super::{ReplayDone, ReplayTask, complete, spawn_with};
use crate::cli::InvestigationCommand;
use crate::commands::Replay;
use crate::interactive::session::state::SessionState;
use crate::interactive::session_prompt::StatusView;
use crate::interactive::tui::application::tests_support::{idle_app, unused_runtime};
use crate::interactive::tui::capture::{CapturedResult, accounted_bytes};
use crate::interactive::tui::loop_tick::tick_workers;
use crate::interactive::tui::worker_permits::{WorkerPermit, test_permit_lock_async};
use crate::render::{RenderFormat, TerminalEvent};
use crate::slash::{ExportMode, ExportRequest};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use saya_store::{FsSessionStore, SqliteStateStore};
use saya_types::{
    EvidenceSource, ExecutionEvidence, ExecutionEvidenceArgs, QueryResult, ResultScope, SqlDialect,
};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

// -- fixtures ---------------------------------------------------------------

const STARTED_UNIX_MS: i64 = 1_790_000_000_000;

fn sample_result() -> QueryResult {
    QueryResult {
        columns: vec!["one".to_string()],
        rows: vec![serde_json::json!([1])],
        row_count: 1,
        truncated: false,
        executed_sql: "SELECT 1 AS one".to_string(),
    }
}

fn saved_evidence() -> ExecutionEvidence {
    saved_evidence_for(&sample_result())
}

fn saved_evidence_for(result: &QueryResult) -> ExecutionEvidence {
    ExecutionEvidence::for_result(
        result,
        ExecutionEvidenceArgs {
            execution_id: "xabc-1".to_string(),
            connection_label: "local".to_string(),
            connection_identity: None,
            dialect: SqlDialect::Sqlite,
            max_rows: 100,
            started_unix_ms: STARTED_UNIX_MS,
            finished_unix_ms: STARTED_UNIX_MS + 5,
            source: EvidenceSource::SavedInvestigation {
                id: "recent-orders-abcdef01".to_string(),
                revision: 1,
            },
        },
    )
}

fn replay() -> Replay {
    Replay {
        result: sample_result(),
        evidence: saved_evidence(),
        sql: "SELECT 1 AS one".to_string(),
        connection: "local".to_string(),
    }
}

fn run_command(id: &str) -> InvestigationCommand {
    InvestigationCommand::Run {
        id: id.to_string(),
        connection: None,
        revalidate: false,
        report: None,
        rows: None,
        overwrite: false,
        params: Vec::new(),
    }
}

fn replay_task(id: &str) -> ReplayTask {
    ReplayTask {
        id: id.to_string(),
        command: run_command(id),
        format: RenderFormat::Text,
        // The gated bodies never run the operation, so a lazy empty-path
        // store is the right fixture here; the end-to-end test composes the
        // real one.
        state_db: SqliteStateStore::new(PathBuf::new()),
        cancellation: saya_agent::CancellationToken::new(),
    }
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "saya-replay-task-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("replay test dir");
    dir
}

fn session_store(root: &Path) -> FsSessionStore {
    FsSessionStore::new(root)
}

fn done(code: i32, text: &str, replay: Option<Replay>) -> ReplayDone {
    ReplayDone {
        code,
        text: text.to_string(),
        replay,
    }
}

/// A replay body gated behind a release signal: it waits for `release`,
/// hands the pre-built outcome to the channel, then signals `handed` — so
/// once `handed` is observed the message is either delivered or (after a
/// detach) failed on a dropped receiver. The test never guesses timing.
fn gated(done: ReplayDone) -> (Gated, impl FnOnce(&Sender<ReplayDone>) + Send + 'static) {
    let (release, release_rx) = mpsc::channel::<()>();
    let (handed, handed_rx) = mpsc::channel::<()>();
    let body = move |tx: &Sender<ReplayDone>| {
        release_rx.recv().expect("release signal");
        let _ = tx.send(done);
        let _ = handed.send(());
    };
    (
        Gated {
            release,
            handed: handed_rx,
        },
        body,
    )
}

struct Gated {
    release: Sender<()>,
    handed: mpsc::Receiver<()>,
}

/// Puts a running replay on `app` exactly the way the dispatch arm does:
/// receiver, task, dispatch instant, and the status fields the bar reuses.
fn put_running_replay(app: &mut App, rx: mpsc::Receiver<ReplayDone>, id: &str) -> Instant {
    let started = Instant::now();
    app.replay_task = Some((rx, replay_task(id), started));
    app.request.started = Some(started);
    app.request.activity = Some(format!("investigation {id}"));
    started
}

/// Ticks the workers until `applied` holds, asserting every tick is prompt —
/// the event loop must never block on the replay.
fn tick_until(
    app: &mut App,
    store: &FsSessionStore,
    state: &mut SessionState,
    applied: impl Fn(&App) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let tick = Instant::now();
        tick_workers(app, store, state);
        assert!(
            tick.elapsed() < Duration::from_millis(250),
            "tick_workers must never block on the replay (took {:?})",
            tick.elapsed()
        );
        if applied(app) {
            return;
        }
        assert!(Instant::now() < deadline, "the replay never settled");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn last_block(transcript: &Transcript, kind: BlockKind) -> Option<String> {
    transcript
        .blocks()
        .iter()
        .rev()
        .find(|b| b.kind == kind)
        .map(|b| b.text.clone())
}

// -- the loop-level behaviour -----------------------------------------------

include!("replay_task_tests/background_lifecycle.rs");

include!("replay_task_tests/capture_size_and_render.rs");

include!("replay_task_tests/failed_outcomes.rs");

include!("replay_task_tests/busy_status.rs");

// -- the pinned export-refresh behaviour ------------------------------------

include!("replay_task_tests/detached_export.rs");

// -- the worker end to end --------------------------------------------------

include!("replay_task_tests/worker_delivery.rs");

// -- detach cancels the replay's token (FIX-S1d) -----------------------------

include!("replay_task_tests/token_lifecycle.rs");
