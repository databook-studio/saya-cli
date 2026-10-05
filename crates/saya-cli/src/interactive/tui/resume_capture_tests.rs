//! E2-4b follow-up (P1): a successful TUI resume is a context change, so the
//! PREVIOUS session's ephemeral SQL capture (`App.captured`) and last query
//! (`App.last_query`) are cleared on the success branches only. Mirrors the
//! real fixtures: the real `/sql` worker turn seeds the capture
//! (session_privacy_receipt_tests.rs), the real picker resume runs
//! `tick_resume`, and the refusal holds the resumed id's lock
//! (session_runtime_question_tests.rs D4).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use saya_agent::{ApprovalPolicy, CancellationToken};
use saya_store::{FsSessionStore, SessionStore};
use saya_types::DatabaseProfile;

use crate::config::runtime::RuntimeConfig;
use crate::interactive::session_resume::block_on;
use crate::interactive::session_runtime::SessionRuntime;
use crate::interactive::session_state::SessionState;
use crate::interactive::tui::application::SecondSqlDecision;
use crate::interactive::tui::dispatch::{Dispatch, dispatch};
use crate::interactive::tui::loop_tick::resume::tick_resume;
use crate::interactive::tui::loop_tick::workers::tick_workers;
use crate::interactive::tui::sql_task;
use crate::interactive::tui::types::App;
use crate::interactive::tui::ui_snapshot_tests::{empty_app, unused_runtime};
use crate::interactive::tui::worker_permits::test_permit_lock;
use crate::render::RenderFormat;

/// A planted row VALUE returned by the real worker turn: it exists only in
/// one SQLite cell, so a surviving capture is provably the old session's.
const MARKER: &str = "PLANTED-ROW-VALUE-9e2a";

/// Creates the one-cell SQLite table the worker turn queries.
fn seed_sqlite(path: &Path, value: &str) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("seed runtime");
    runtime.block_on(async {
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true);
        let pool = sqlx::SqlitePool::connect_with(options)
            .await
            .expect("seed database creates");
        sqlx::query("CREATE TABLE t (value TEXT)")
            .execute(&pool)
            .await
            .expect("seed table");
        sqlx::query("INSERT INTO t (value) VALUES (?)")
            .bind(value)
            .execute(&pool)
            .await
            .expect("seed planted row");
    });
}

/// One real round trip: a live session runtime, a real `/sql` worker turn
/// that seeds `App.captured`/`App.last_query`, and the real picker resume.
struct Seed {
    app: App,
    state: SessionState,
    session: SessionRuntime,
    runtime: Arc<RuntimeConfig>,
    store: FsSessionStore,
    root: PathBuf,
}

impl Seed {
    fn new(tag: &str, id: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("saya-resume-capture-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("the resume-capture root");
        let db = root.join("planted.sqlite3");
        seed_sqlite(&db, MARKER);
        let mut config = std::sync::Arc::unwrap_or_clone(unused_runtime());
        config.connections.profiles.insert(
            "analytics".to_string(),
            DatabaseProfile::Sqlite {
                path: db.display().to_string(),
                read_only: true,
            },
        );
        config.resolved.max_rows = 1;
        let runtime = Arc::new(config);
        let mut app = empty_app();
        app.runtime = Arc::clone(&runtime);
        let session = SessionRuntime::acquire(
            &runtime,
            Some(&root),
            true,
            None,
            id,
            ApprovalPolicy::Ask,
            &root.join("session-dirs"),
        )
        .expect("the live session acquires");
        let state = SessionState::new(id, None, "test-model");
        Seed {
            app,
            state,
            session,
            runtime,
            store: FsSessionStore::new(root.join("store")),
            root,
        }
    }

    /// The real `dispatch`, called with the fixture's pieces exactly as the
    /// live loop does.
    fn dispatch_line(&mut self, line: &str) -> Dispatch {
        dispatch(
            line,
            &mut self.app.transcript,
            &self.app.profiles,
            &mut self.state,
            self.runtime.as_ref(),
            &self.store,
            &self.app.state_db,
            RenderFormat::Text,
            &mut self.app.last_query,
            &self.app.captured,
            self.app.agent_captures.gap,
            &mut self.session,
        )
    }

    /// Ticks the real worker poll until `ready`, with a bounded deadline.
    fn tick(&mut self, what: &str, ready: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready(&self.app) {
            assert!(Instant::now() < deadline, "{what}");
            tick_workers(&mut self.app, &self.store, &mut self.state);
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// The real direct-SQL turn: `/connect` then `/sql`, admitted and spawned
    /// exactly as `loop_tick::pending` does, polled to a captured result.
    fn capture_marker_query(&mut self) {
        assert!(
            matches!(self.dispatch_line("/connect analytics"), Dispatch::Handled),
            "the profile connects"
        );
        let Dispatch::SqlTask(task) = self.dispatch_line("/sql SELECT * FROM t") else {
            panic!("/sql dispatches its worker task");
        };
        let _pool = test_permit_lock();
        match self.app.admit_second_sql() {
            SecondSqlDecision::Start(permit) => {
                let started = Instant::now();
                let cancellation = CancellationToken::new();
                self.app.sql_task = Some((
                    sql_task::spawn(
                        permit,
                        Arc::clone(&self.runtime),
                        task.clone(),
                        cancellation.clone(),
                    ),
                    task,
                    started,
                    cancellation,
                ));
            }
            SecondSqlDecision::Reject(message) => panic!("admission refused the query: {message}"),
        }
        self.tick("the /sql worker never completed", |app| {
            app.captured.is_some()
        });
        let capture = self
            .app
            .captured
            .as_ref()
            .expect("a successful /sql is captured");
        assert_eq!(
            capture.result.rows[0][0], MARKER,
            "the capture holds the planted row value"
        );
        assert!(
            self.app.last_query.is_some(),
            "the successful /sql promoted a selectable last query"
        );
    }
}

/// Gives the loop's background save a moment to land before cleanup, as the
/// D4 fixture does.
fn wait_for_background_save(store_root: &Path) {
    for _ in 0..200 {
        if std::fs::read_dir(store_root).is_ok_and(|entries| entries.count() > 0) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// (a) PICKER success: after a real `/sql` capture, a successful
/// `tick_resume` clears BOTH the captured result and the last query, and the
/// resume message is shown.
#[test]
fn the_picker_resume_clears_the_previous_sessions_capture_and_last_query() {
    let mut fx = Seed::new("success", "cap-live");
    fx.capture_marker_query();
    block_on(
        fx.store
            .save(SessionState::new("resumed-ok", None, "test-model").redacted()),
    )
    .expect("the resumed record saves");
    fx.app.overlays.pending_resume = Some("resumed-ok".into());
    tick_resume(
        &mut fx.app,
        &fx.store,
        &mut fx.state,
        fx.runtime.as_ref(),
        &mut fx.session,
    );
    assert!(
        fx.app
            .transcript
            .blocks()
            .iter()
            .any(|block| block.text.contains("Resumed session resumed-ok")),
        "the picker resume said it resumed, not {:?}",
        fx.app
            .transcript
            .blocks()
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        fx.app.captured.is_none(),
        "the resumed session inherited the previous session's capture"
    );
    assert!(
        fx.app.last_query.is_none(),
        "the resumed session inherited the previous session's last query"
    );
    let root = fx.root.clone();
    drop(fx);
    wait_for_background_save(&root.join("store"));
    let _ = std::fs::remove_dir_all(&root);
}

/// (b) PICKER refusal: with the resumed id's lock held (the D4 technique),
/// the refused resume changes NOTHING — both the real captured result and
/// the last query survive.
#[test]
fn the_refused_picker_resume_keeps_the_previous_sessions_capture_and_last_query() {
    let mut fx = Seed::new("refusal", "cap-held-live");
    fx.capture_marker_query();
    let blocker = SessionRuntime::acquire(
        &fx.runtime,
        None,
        false,
        None,
        "resumed-held",
        ApprovalPolicy::Ask,
        &fx.root.join("session-dirs"),
    )
    .expect("the blocker session acquires");
    block_on(
        fx.store
            .save(SessionState::new("resumed-held", None, "test-model").redacted()),
    )
    .expect("the resumed record saves");
    fx.app.overlays.pending_resume = Some("resumed-held".into());
    tick_resume(
        &mut fx.app,
        &fx.store,
        &mut fx.state,
        fx.runtime.as_ref(),
        &mut fx.session,
    );
    assert!(
        fx.app
            .transcript
            .blocks()
            .iter()
            .any(|block| block.text.contains("already running")),
        "the refused resume said why, not {:?}",
        fx.app
            .transcript
            .blocks()
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
    );
    let capture = fx
        .app
        .captured
        .as_ref()
        .expect("the refused resume kept the capture");
    assert_eq!(
        capture.result.rows[0][0], MARKER,
        "the kept capture is the real captured result"
    );
    assert!(
        fx.app.last_query.is_some(),
        "the refused resume kept the last query"
    );
    let root = fx.root.clone();
    drop(blocker);
    drop(fx);
    wait_for_background_save(&root.join("store"));
    let _ = std::fs::remove_dir_all(&root);
}

/// FIX-S1e (a): a resume must not start while a request is running. With a
/// compaction in flight, `tick_resume` refuses: the session id is unchanged,
/// the pending id is consumed, the transcript says why, nothing resumed.
#[test]
fn a_resume_is_refused_while_a_request_is_running() {
    let mut fx = Seed::new("busy", "busy-live");
    block_on(
        fx.store
            .save(SessionState::new("resumed-busy", None, "test-model").redacted()),
    )
    .expect("the resumed record saves");
    let before = fx.state.id.clone();
    let (_tx, rx) = std::sync::mpsc::channel();
    fx.app.compact_task = Some(rx);
    fx.app.overlays.pending_resume = Some("resumed-busy".into());
    tick_resume(
        &mut fx.app,
        &fx.store,
        &mut fx.state,
        fx.runtime.as_ref(),
        &mut fx.session,
    );
    assert_eq!(fx.state.id, before, "the busy resume changed the session");
    assert!(
        fx.app.overlays.pending_resume.is_none(),
        "the busy resume kept the pending id"
    );
    let blocks: Vec<String> = fx
        .app
        .transcript
        .blocks()
        .iter()
        .map(|b| b.text.clone())
        .collect();
    assert!(
        blocks.iter().any(|text| text.contains("still running")),
        "the refused resume said why, not {:?}",
        blocks
    );
    assert!(
        !blocks.iter().any(|text| text.contains("Resumed session")),
        "the busy resume resumed the session anyway, not {:?}",
        blocks
    );
    let root = fx.root.clone();
    drop(fx);
    wait_for_background_save(&root.join("store"));
    let _ = std::fs::remove_dir_all(&root);
}

/// FIX-S1e (b): a compaction is part of the one-query-at-a-time resource,
/// so `admit_second_replay` refuses while one runs instead of starting a
/// replay beside it (it would clobber the status and capture fields).
#[test]
fn a_second_replay_is_refused_while_a_compaction_runs() {
    let mut fx = Seed::new("replay", "replay-live");
    let (_tx, rx) = std::sync::mpsc::channel();
    fx.app.compact_task = Some(rx);
    assert!(
        matches!(fx.app.admit_second_replay(), SecondSqlDecision::Reject(_)),
        "the replay was admitted beside a running compaction"
    );
    let root = fx.root.clone();
    drop(fx);
    let _ = std::fs::remove_dir_all(&root);
}
