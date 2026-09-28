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
use crate::interactive::session_prompt::StatusView;
use crate::interactive::session_state::SessionState;
use crate::interactive::tui::application::tests_support::{idle_app, unused_runtime};
use crate::interactive::tui::capture::{CapturedResult, accounted_bytes, unix_now_ms};
use crate::interactive::tui::loop_tick::tick_workers;
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
    }
}

fn replay_task(id: &str) -> ReplayTask {
    ReplayTask {
        id: id.to_string(),
        command: run_command(id),
        format: RenderFormat::Text,
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

/// Dispatch returns the replay task at once (nothing ran, nothing pushed),
/// and while the worker runs the loop keeps ticking: a poll applies nothing,
/// keys still edit the input, and the completion lands only on a later tick.
#[test]
fn replay_runs_in_background_and_loop_keeps_ticking() {
    // Dispatch: the adapter hands back a task immediately.
    let mut transcript = Transcript::new();
    let runtime = unused_runtime();
    let store = SqliteStateStore::new(PathBuf::new());
    let outcome = run_investigation(
        &mut transcript,
        &runtime,
        &store,
        RenderFormat::Text,
        &run_command("recent-orders"),
        &None,
    );
    let Some(Dispatch::ReplayTask(task)) = outcome else {
        panic!("a run dispatches a replay task");
    };
    assert_eq!(task.id, "recent-orders");
    assert!(
        transcript.blocks().is_empty(),
        "dispatch runs nothing inline: {:?}",
        transcript.blocks()
    );

    // The loop: a gated worker — the replay is in flight, so the app is busy.
    let mut app = idle_app();
    let mut state = SessionState::new("test", None, "model");
    let store = session_store(temp_dir("ticking").as_path());
    let (gate, body) = gated(done(0, "the replay result", Some(replay())));
    put_running_replay(&mut app, spawn_with(body), "recent-orders");

    // While the worker waits, polls apply nothing and keys keep working.
    tick_workers(&mut app, &store, &mut state);
    handle_key(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
    assert_eq!(
        app.input.text(),
        "x",
        "keys still work while the replay runs"
    );
    assert!(app.is_busy(), "a running replay reads as busy");

    // Release: the completion lands on a later tick, and the loop stays
    // prompt until it does.
    gate.release.send(()).expect("release the replay worker");
    gate.handed
        .recv_timeout(Duration::from_secs(5))
        .expect("the worker handed off its outcome");
    tick_until(&mut app, &store, &mut state, |app| {
        app.replay_task.is_none()
    });
    // The typed replay drives the completion: a Table block like a direct
    // /sql result, never the captured CLI text (whose tabs a System block
    // strips).
    assert!(
        last_block(&app.transcript, BlockKind::Table).is_some_and(|text| text.contains("│ one │")),
        "the replay result renders as a table on completion"
    );
    assert!(
        !app.transcript
            .blocks()
            .iter()
            .any(|b| b.text.contains('\t')),
        "the captured tab-separated text is never pushed: {:?}",
        app.transcript.blocks()
    );
    assert!(!app.is_busy());
}

/// Esc detaches a running replay exactly like a SQL task: the receiver is
/// dropped, the message is honest (may still be running server-side, result
/// discarded — never "cancelled"), and the late completion changes nothing.
#[test]
fn detached_replay_late_completion_changes_nothing() {
    let mut app = idle_app();
    let (gate, body) = gated(done(0, "the replay result", Some(replay())));
    let rx = spawn_with(body);
    put_running_replay(&mut app, rx, "recent-orders");

    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(
        app.replay_task.is_none(),
        "detach clears the in-flight replay"
    );
    assert!(!app.is_busy());
    assert_eq!(
        app.request.started, None,
        "the bar's status fields are released"
    );
    assert_eq!(app.request.activity, None);
    let message = last_block(&app.transcript, BlockKind::System).expect("detach is said");
    assert!(
        message.contains("Detached the running investigation recent-orders"),
        "the detach message names the investigation: {message}"
    );
    assert!(
        message.contains("may still be running on the server") && message.contains("discarded"),
        "the detach message is honest about the server-side query: {message}"
    );
    assert!(
        !message.contains("cancel"),
        "detach never claims cancellation: {message}"
    );

    // The worker finishes late; its outcome lands on a dropped channel.
    gate.release.send(()).expect("release the replay worker");
    gate.handed
        .recv_timeout(Duration::from_secs(5))
        .expect("the worker handed off (to a dropped receiver)");
    let mut state = SessionState::new("test", None, "model");
    tick_workers(
        &mut app,
        &session_store(temp_dir("detach").as_path()),
        &mut state,
    );

    // Nothing changed: the transcript holds only the detach message, and the
    // capture and selectable query are untouched.
    assert!(
        app.transcript.blocks().len() == 1,
        "a late completion after detach must not touch the transcript: {:?}",
        app.transcript.blocks()
    );
    assert!(app.captured.is_none(), "a detached replay never captures");
    assert!(app.last_query.is_none(), "a detached replay never promotes");
}

/// An oversize replay honours the same capture budget the direct-/sql path
/// uses, through the same test-sized walk: over budget the result is not
/// captured (any previous capture is cleared), one visible system line names
/// the way out, and — the query having succeeded — the replay is still the
/// latest selectable query.
#[test]
fn oversize_replay_is_not_captured_and_says_so() {
    let big = QueryResult {
        columns: vec!["text".to_string()],
        rows: vec![serde_json::json!(["x".repeat(200)])],
        row_count: 1,
        truncated: false,
        executed_sql: "SELECT text FROM t".to_string(),
    };
    // The existing test-sized budget seam (`capture::accounted_bytes_within`,
    // the walk `capture_within` takes): a small budget refuses the big result
    // and admits the small one.
    let accounted = super::super::capture::accounted_bytes_within(&big, 64);
    assert!(
        accounted.is_none(),
        "precondition: the big result refuses the budget"
    );
    assert!(
        super::super::capture::accounted_bytes_within(&sample_result(), 64).is_some(),
        "precondition: the small result stays within it"
    );
    let replay = Replay {
        result: big,
        evidence: saved_evidence(),
        sql: "SELECT text FROM t".to_string(),
        connection: "local".to_string(),
    };
    let mut transcript = Transcript::new();
    let mut last_query: Option<LastQuery> = None;
    let mut captured = Some(CapturedResult {
        result: sample_result(),
        evidence: saved_evidence(),
    });
    complete(
        done(0, "the replay output", Some(replay)),
        accounted,
        &mut transcript,
        &mut last_query,
        &mut captured,
    );
    // The query succeeded: the replay is still selectable.
    let last_query = last_query
        .as_ref()
        .expect("the oversize replay is still selectable");
    assert_eq!(last_query.sql, "SELECT text FROM t");
    assert_eq!(last_query.connection.as_deref(), Some("local"));
    // Over budget: not captured, the previous capture cleared, one visible line.
    assert!(captured.is_none(), "an oversize replay is not captured");
    let note = last_block(&transcript, BlockKind::System).expect("the refusal is visible");
    assert!(note.contains("Result not captured"), "{note}");
    assert!(note.contains("larger than 32 MiB"), "{note}");
    assert!(
        note.contains("/export --refresh will re-run it"),
        "the refusal names the way out: {note}"
    );
}

/// A successful completion renders the typed replay as one Table block (the
/// captured CLI text is dropped — its tabs would be stripped), captures the
/// replay (source saved investigation, scope full), and sets the latest
/// selectable query to the replay's SQL and connection.
#[test]
fn replay_completion_sets_capture_and_last_query() {
    let mut app = idle_app();
    let mut state = SessionState::new("test", None, "model");
    let store = session_store(temp_dir("capture").as_path());
    let (tx, rx) = mpsc::channel();
    tx.send(done(0, "table + evidence text", Some(replay())))
        .expect("pre-delivered completion");
    put_running_replay(&mut app, rx, "recent-orders");

    tick_workers(&mut app, &store, &mut state);

    assert!(app.replay_task.is_none() && !app.is_busy());
    assert!(
        last_block(&app.transcript, BlockKind::System).is_none(),
        "the captured CLI text is not pushed: {:?}",
        app.transcript.blocks()
    );
    let block =
        last_block(&app.transcript, BlockKind::Table).expect("the replay renders as a table block");
    assert!(
        block.contains("│ one │"),
        "the result table renders: {block}"
    );
    assert!(
        block.contains("saved investigation: local"),
        "the evidence line rides the table block: {block}"
    );
    let captured = app
        .captured
        .as_ref()
        .expect("a successful replay is captured");
    assert!(
        matches!(
            captured.evidence.source,
            EvidenceSource::SavedInvestigation { .. }
        ),
        "the capture is a saved-investigation result: {:?}",
        captured.evidence.source
    );
    assert_eq!(
        captured.evidence.scope,
        ResultScope::Full,
        "a replay is a full read"
    );
    assert_eq!(captured.result.row_count, 1);
    let last_query = app.last_query.as_ref().expect("the replay is selectable");
    assert_eq!(last_query.sql, "SELECT 1 AS one");
    assert_eq!(
        last_query.connection.as_deref(),
        Some("local"),
        "the selectable query names the connection the replay ran on"
    );
    assert_eq!(
        app.request.started, None,
        "the bar's status fields are released"
    );
    assert_eq!(app.request.activity, None);
}

/// A successful replay renders its rows exactly like a direct /sql result:
/// one Table block — the box table, the scope line, the evidence line — and
/// never the captured CLI text, whose tab-separated rows a System block
/// strips into concatenated words (`regionjoined_rows…`).
#[test]
fn successful_replay_renders_a_table_block_with_evidence() {
    let result = QueryResult {
        columns: vec![
            "region".to_string(),
            "joined_rows".to_string(),
            "distinct_orders".to_string(),
        ],
        rows: vec![serde_json::json!(["north", 8, 359])],
        row_count: 1,
        truncated: false,
        executed_sql: "SELECT region, joined_rows, distinct_orders FROM orders".to_string(),
    };
    let evidence = saved_evidence_for(&result);
    let accounted = accounted_bytes(&result);
    let replay = Replay {
        result,
        evidence,
        sql: "SELECT region, joined_rows, distinct_orders FROM orders".to_string(),
        connection: "local".to_string(),
    };
    // The captured CLI text: tab-separated rows and the evidence line — the
    // non-TTY render whose tabs a System block strips.
    let captured_text = "region\tjoined_rows\tdistinct_orders\n\
                         north\t8\t359\n\
                         saved investigation: local · 1 rows · exec xabc-1 · full result";
    let mut transcript = Transcript::new();
    let mut last_query: Option<LastQuery> = None;
    let mut captured: Option<CapturedResult> = None;
    complete(
        done(0, captured_text, Some(replay)),
        accounted,
        &mut transcript,
        &mut last_query,
        &mut captured,
    );
    let block = transcript
        .blocks()
        .last()
        .expect("the completion pushes one block");
    assert_eq!(
        block.kind,
        BlockKind::Table,
        "the rows render as a table block: {:?}",
        transcript.blocks()
    );
    let text = block.text.as_str();
    // Columns separated by the table's separator — never the concatenated
    // words the tab-stripped captured text produced.
    assert!(
        text.contains("│ region │ joined_rows │ distinct_orders │"),
        "header cells are separated by the table's separator: {text}"
    );
    assert!(
        !text.contains("regionjoined_rows"),
        "no tab-stripped concatenation: {text}"
    );
    assert!(
        text.contains("│ north") && text.contains(" 8 │") && !text.contains("north8"),
        "the row renders boxed, not tab-stripped: {text}"
    );
    // The scope line, built exactly as the /sql path builds it.
    assert!(
        text.contains("from local · SELECT region, joined_rows, distinct_orders FROM orders"),
        "the scope line names the connection and SQL: {text}"
    );
    // The evidence line under the table.
    assert!(
        text.contains("saved investigation: local") && text.contains("full result"),
        "the evidence line rides the table block: {text}"
    );
    // The captured CLI text is dropped entirely: no tab ever reaches a block.
    assert!(
        transcript.blocks().iter().all(|b| !b.text.contains('\t')),
        "the captured tab-separated text is never pushed: {:?}",
        transcript.blocks()
    );
    assert_eq!(
        transcript.blocks().len(),
        1,
        "the table block replaces the captured text: {:?}",
        transcript.blocks()
    );
    let last_query = last_query.as_ref().expect("the replay is selectable");
    assert_eq!(
        last_query.sql,
        "SELECT region, joined_rows, distinct_orders FROM orders"
    );
    assert_eq!(last_query.connection.as_deref(), Some("local"));
    let captured = captured
        .as_ref()
        .expect("within budget, the replay is captured");
    assert_eq!(captured.result.row_count, 1);
}

/// A failed replay pushes an error block and leaves the previous capture and
/// selectable query exactly as they were.
#[test]
fn failed_replay_keeps_previous_capture() {
    let mut app = idle_app();
    let mut state = SessionState::new("test", None, "model");
    let store = session_store(temp_dir("failed").as_path());
    let previous = CapturedResult {
        result: sample_result(),
        evidence: saved_evidence(),
    };
    app.captured = Some(CapturedResult {
        result: previous.result.clone(),
        evidence: previous.evidence.clone(),
    });
    app.last_query = Some(LastQuery {
        sql: "SELECT 2 AS two".to_string(),
        connection: Some("other".to_string()),
    });
    let (tx, rx) = mpsc::channel();
    tx.send(done(2, "no investigation nope", None))
        .expect("pre-delivered failure");
    put_running_replay(&mut app, rx, "nope");

    tick_workers(&mut app, &store, &mut state);

    let block = last_block(&app.transcript, BlockKind::Error).expect("the failure is said");
    assert_eq!(block, "no investigation nope");
    let captured = app
        .captured
        .as_ref()
        .expect("the previous capture survives");
    assert_eq!(
        captured.result.executed_sql, previous.result.executed_sql,
        "a failed replay never replaces the capture"
    );
    let last_query = app
        .last_query
        .as_ref()
        .expect("the previous query survives");
    assert_eq!(last_query.sql, "SELECT 2 AS two");
    assert_eq!(last_query.connection.as_deref(), Some("other"));
}

/// A non-zero exit that still carries the typed replay — a successful
/// execution whose later step refused, unreachable from the TUI's slash run
/// (which passes no report flags) — keeps today's behaviour: the text as an
/// error block, and the replay still promoted and captured, since the query
/// did run.
#[test]
fn failed_code_with_typed_replay_still_promotes_and_captures() {
    let mut transcript = Transcript::new();
    let mut last_query: Option<LastQuery> = None;
    let mut captured: Option<CapturedResult> = None;
    complete(
        done(2, "the report write refused", Some(replay())),
        accounted_bytes(&sample_result()),
        &mut transcript,
        &mut last_query,
        &mut captured,
    );
    let block = last_block(&transcript, BlockKind::Error).expect("the failure is said");
    assert_eq!(block, "the report write refused");
    assert!(
        last_block(&transcript, BlockKind::System).is_none(),
        "no system block on a failure: {:?}",
        transcript.blocks()
    );
    let last_query = last_query
        .as_ref()
        .expect("the run's replay is still promoted");
    assert_eq!(last_query.sql, "SELECT 1 AS one");
    assert_eq!(last_query.connection.as_deref(), Some("local"));
    assert!(captured.is_some(), "the run's result is still captured");
}

/// A successful replay whose captured output and stderr are both empty
/// pushes no block: an empty System block is a transcript glitch, not a
/// message.
#[test]
fn successful_replay_with_no_output_pushes_no_block() {
    let mut transcript = Transcript::new();
    let mut last_query: Option<LastQuery> = None;
    let mut captured: Option<CapturedResult> = None;
    complete(
        done(0, "", None),
        None,
        &mut transcript,
        &mut last_query,
        &mut captured,
    );
    assert!(
        transcript.blocks().is_empty(),
        "an empty output is no block at all: {:?}",
        transcript.blocks()
    );
    assert!(last_query.is_none() && captured.is_none());
}

/// The same skip for a failure: empty text is no block of any kind — the
/// completion never pushes a block it has nothing to say in.
#[test]
fn failed_replay_with_no_output_pushes_no_block() {
    let mut transcript = Transcript::new();
    let mut last_query: Option<LastQuery> = None;
    let mut captured: Option<CapturedResult> = None;
    complete(
        done(2, "", None),
        None,
        &mut transcript,
        &mut last_query,
        &mut captured,
    );
    assert!(
        transcript.blocks().is_empty(),
        "an empty failure text is no block at all: {:?}",
        transcript.blocks()
    );
}

/// While a replay runs, a second replay or /sql submitted is queued (the
/// `is_busy` gate), a task reaching the dispatch handler is refused by the
/// backstop guard — in both directions — and Esc detaches. A second replay
/// is never silently dropped or started over the first.
#[test]
fn second_replay_while_running_is_refused_or_queued() {
    let mut app = idle_app();
    let (gate, body) = gated(done(0, "the replay result", None));
    let rx = spawn_with(body);
    put_running_replay(&mut app, rx, "recent-orders");

    // A second replay submitted while one runs is queued, not dispatched.
    app.input.set_text("/investigation run other-one");
    app.submit();
    assert_eq!(
        app.pending.as_deref(),
        Some("/investigation run other-one"),
        "a second replay is queued behind the running one"
    );
    // A second /sql is queued the same way (one slot: it replaces the queue).
    app.input.set_text("/sql SELECT 1");
    app.submit();
    assert_eq!(app.pending.as_deref(), Some("/sql SELECT 1"));
    assert!(
        app.replay_task.is_some(),
        "queueing never disturbed the running replay"
    );

    // The backstop guard refuses a task reaching the handler — both ways.
    assert!(matches!(
        app.admit_second_replay(),
        SecondSqlDecision::Reject(_)
    ));
    assert!(matches!(
        app.admit_second_sql(),
        SecondSqlDecision::Reject(_)
    ));

    // Esc detaches through the real key path.
    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.replay_task.is_none());

    // Ctrl+C detaches too, before reaching the quit arming.
    let (gate2, body2) = gated(done(0, "the replay result", None));
    put_running_replay(&mut app, spawn_with(body2), "recent-orders");
    handle_key(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(
        app.replay_task.is_none(),
        "Ctrl+C detaches a running replay"
    );
    assert!(!app.ctrl_c_armed, "detach is not a quit arming");
    assert!(!app.should_quit);
    let _ = gate.release.send(());
    let _ = gate2.release.send(());
}

/// The replay is visible: the status bar shows the spinner and names the
/// investigation — "running investigation <id>" — never "thinking".
#[test]
fn running_replay_is_visible_in_the_status_bar() {
    const SPINNER_CHARS: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

    let mut app = idle_app();
    let started = Instant::now();
    let (gate, body) = gated(done(0, "the replay result", None));
    app.replay_task = Some((spawn_with(body), replay_task("recent-orders"), started));
    app.request.started = Some(started);
    app.request.activity = Some("investigation recent-orders".into());
    let status = StatusView {
        profile: "local".into(),
        included: Vec::new(),
        model: "qwen".into(),
        approval_mode: "read-only".into(),
        agent_mode: "build".into(),
        workspace_root: None,
        sharing_on: true,
        host_composed: false,
        denied_programs: Vec::new(),
        task_summary: None,
    };
    let backend = ratatui::backend::TestBackend::new(80, 24);
    let mut terminal = ratatui::Terminal::new(backend).expect("test backend builds");
    terminal
        .draw(|frame| crate::interactive::tui::ui::draw(frame, &app, &status))
        .expect("draw completes");
    let buffer = format!("{}", terminal.backend());
    assert!(
        buffer.contains("running investigation recent-orders"),
        "the bar names the running investigation:\n{buffer}"
    );
    assert!(
        !buffer.contains("thinking"),
        "a replay must not be labelled 'thinking':\n{buffer}"
    );
    assert!(
        SPINNER_CHARS.iter().any(|c| buffer.contains(c)),
        "a spinner frame should be present:\n{buffer}"
    );
    let _ = gate.release.send(());
}

// -- the pinned export-refresh behaviour ------------------------------------

/// A detached `/export --refresh` must not write its file when its result
/// arrives late: detach drops the receiver, so the completion — the only
/// place the export is written — never runs. The undetached control proves
/// the file can be written at all, so the refusal is the detach, not the
/// fixture.
#[test]
fn detached_refresh_export_writes_no_file() {
    fn export_task(path: &Path) -> SqlTask {
        SqlTask {
            profile: Some("local".to_string()),
            sql: "SELECT 1 AS one".to_string(),
            followup: Followup::Export {
                request: ExportRequest {
                    mode: Some(ExportMode::Refresh),
                    overwrite: false,
                    path: path.to_string_lossy().into_owned(),
                },
            },
            started_unix_ms: unix_now_ms(),
        }
    }

    let dir = temp_dir("refresh-export");
    let path = dir.join("out.csv");

    // The control: without a detach the completion writes the file.
    {
        let mut app = idle_app();
        let mut state = SessionState::new("test", None, "model");
        let store = session_store(&dir);
        let (tx, rx) = mpsc::channel();
        app.sql_task = Some((rx, export_task(&path), Instant::now()));
        tx.send(TerminalEvent::QueryResult {
            result: sample_result(),
        })
        .expect("the late result arrives");
        tick_workers(&mut app, &store, &mut state);
        assert!(
            path.exists(),
            "control: the undetached refresh writes the file"
        );
    }
    let _ = std::fs::remove_file(&path);

    // The pinned behaviour: detach first, result arrives late, no file.
    {
        let mut app = idle_app();
        let mut state = SessionState::new("test", None, "model");
        let store = session_store(&dir);
        let (tx, rx) = mpsc::channel();
        app.sql_task = Some((rx, export_task(&path), Instant::now()));
        app.request.started = Some(Instant::now());
        app.request.activity = Some("query".into());
        app.detach_sql_task();
        // The worker's result lands on the dropped receiver — the send the
        // real worker makes (`let _ = tx.send(event)`) fails exactly like this.
        tx.send(TerminalEvent::QueryResult {
            result: sample_result(),
        })
        .expect_err("the receiver was dropped by the detach");
        tick_workers(&mut app, &store, &mut state);
        assert!(
            !path.exists(),
            "a detached refresh must never write its export file"
        );
        let system = last_block(&app.transcript, BlockKind::System).expect("detach is said");
        assert!(
            system.contains("Detached the running query"),
            "the detach message is present: {system}"
        );
        assert!(
            last_block(&app.transcript, BlockKind::System)
                .is_none_or(|text| !text.contains("Exported")),
            "no export success is reported after detach"
        );
    }
}

// -- the worker end to end --------------------------------------------------

/// The production worker end to end: it runs the shared typed operation
/// (`run_investigation_outcome`) on its own thread with the output captured
/// there, and the loop's completion captures the typed replay — source
/// saved investigation, scope full — against a real SQLite profile.
#[tokio::test]
async fn replay_worker_runs_the_shared_operation_and_delivers_the_outcome() {
    // The async-aware lock the await points below are held across.
    static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _env = ENV_LOCK.lock().await;
    let root = temp_dir("worker-end-to-end");
    for dir in ["investigations", "config-home", "home"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    let database = root.join("data.sqlite3");
    std::fs::write(&database, b"").unwrap();
    std::fs::write(
        root.join("connections.toml"),
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n",
            database.display()
        ),
    )
    .unwrap();

    let previous_state = std::env::var_os("SAYA_STATE_DB");
    let previous_investigations = std::env::var_os("SAYA_INVESTIGATIONS_DIR");
    // SAFETY: the lock above serializes every test that points the
    // process-global roots at a private directory.
    unsafe { std::env::set_var("SAYA_STATE_DB", root.join("state.sqlite3")) };
    unsafe { std::env::set_var("SAYA_INVESTIGATIONS_DIR", root.join("investigations")) };

    let options = crate::cli::GlobalOptions {
        connections: Some(root.join("connections.toml")),
        ..Default::default()
    };
    let runtime = std::sync::Arc::new(
        crate::config::runtime::load_with_sources(
            &options,
            root.join("config-home").as_path(),
            root.join("config-home").as_path(),
            std::collections::BTreeMap::new(),
        )
        .expect("the test runtime loads"),
    );
    let state_db = SqliteStateStore::new(root.join("state.sqlite3"));

    // Save one investigation through the shared operation first.
    crate::commands::capture_output_start();
    let outcome = crate::commands::run_investigation_outcome(
        InvestigationCommand::Save {
            name: "Recent orders".into(),
            description: None,
            sql: Some("SELECT 1 AS one".into()),
            file: None,
            connection: Some("local".into()),
        },
        &runtime,
        RenderFormat::Text,
        false,
        &state_db,
    )
    .await
    .expect("the save succeeds");
    let (out, err) = crate::commands::capture_output_take();
    assert_eq!(outcome.code, 0, "save stderr: {err}");
    let id = out
        .lines()
        .next()
        .expect("save prints the id")
        .trim()
        .to_string();

    // The real background replay: dispatch through the loop path. The app's
    // runtime is the test runtime — the worker replays against the profile
    // it resolves, not a dummy.
    let mut app = idle_app();
    app.runtime = std::sync::Arc::clone(&runtime);
    let mut state = SessionState::new("test", None, "model");
    let store = session_store(&root);
    app.start_replay(ReplayTask {
        id: id.clone(),
        command: run_command(&id),
        format: RenderFormat::Text,
    });
    assert!(app.is_busy(), "the replay is in flight");
    assert_eq!(
        app.request.activity.as_deref(),
        Some(format!("investigation {id}").as_str()),
        "the spinner names the investigation"
    );

    tick_until(&mut app, &store, &mut state, |app| {
        app.replay_task.is_none()
    });

    let captured = app.captured.as_ref().expect("the replay is captured");
    assert!(matches!(
        captured.evidence.source,
        EvidenceSource::SavedInvestigation { .. }
    ));
    assert_eq!(captured.evidence.scope, ResultScope::Full);
    let last_query = app.last_query.as_ref().expect("the replay is selectable");
    assert_eq!(last_query.sql, "SELECT 1 AS one");
    assert_eq!(last_query.connection.as_deref(), Some("local"));
    let block = last_block(&app.transcript, BlockKind::Table).expect("output pushed");
    assert!(
        block.contains("│ one │"),
        "the shared operation's result renders as a table: {block}"
    );
    assert!(
        block.contains("saved investigation: local"),
        "the shared operation's evidence line is in the output: {block}"
    );
    assert!(
        !app.transcript
            .blocks()
            .iter()
            .any(|b| b.text.contains('\t')),
        "the captured tab-separated CLI text is never pushed: {:?}",
        app.transcript.blocks()
    );

    match previous_state {
        Some(value) => unsafe { std::env::set_var("SAYA_STATE_DB", value) },
        None => unsafe { std::env::remove_var("SAYA_STATE_DB") },
    }
    match previous_investigations {
        Some(value) => unsafe { std::env::set_var("SAYA_INVESTIGATIONS_DIR", value) },
        None => unsafe { std::env::remove_var("SAYA_INVESTIGATIONS_DIR") },
    }
    let _ = std::fs::remove_dir_all(root);
}
