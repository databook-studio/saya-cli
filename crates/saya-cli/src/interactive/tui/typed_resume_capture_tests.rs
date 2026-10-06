//! FIX-S1b: the TYPED `/resume <id>` path (queued prompt -> `dispatch` ->
//! `tick_pending`) retires the PREVIOUS session's ephemeral query state on
//! success only: `App.captured`, `App.last_query`, and the agent-capture
//! gap. Mirrors the real fixtures: the real `/sql` worker turn seeds the
//! capture (session_privacy_receipt_tests.rs), the real typed resume runs
//! `tick_pending` over the queued line, and the refusal holds the resumed
//! id's lock (session_runtime_question_tests.rs D4).
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use saya_agent::{ApprovalPolicy, CancellationToken};
use saya_store::{FsSessionStore, SessionStore};
use saya_types::DatabaseProfile;

use crate::cli::GlobalOptions;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session_resume::block_on;
use crate::interactive::session_runtime::SessionRuntime;
use crate::interactive::session_state::SessionState;
use crate::interactive::tui::application::SecondSqlDecision;
use crate::interactive::tui::capture_agent::CaptureGap;
use crate::interactive::tui::dispatch::{Dispatch, dispatch};
use crate::interactive::tui::loop_tick::pending::tick_pending;
use crate::interactive::tui::loop_tick::workers::tick_workers;
use crate::interactive::tui::sql_task;
use crate::interactive::tui::types::App;
use crate::interactive::tui::ui_snapshot_tests::{empty_app, unused_runtime};
use crate::interactive::tui::worker_permits::test_permit_lock;
use crate::local_metrics::set_enabled;
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

    /// The real queued-prompt drain: the typed `/resume <id>` rides
    /// `tick_pending` exactly as the live loop does (the line is queued and
    /// drained once the gate sees the app idle). The handoff inputs are the
    /// loop's: a no-op `TerminalModes` double (no `/setup` line is ever
    /// drained here, so neither half can run) and default CLI options.
    fn drain_pending(&mut self, line: &str) {
        self.app.pending = Some(line.to_string());
        let mut modes = NoModes;
        let options = GlobalOptions::default();
        let mut setup = crate::interactive::tui::loop_tick::pending::SetupInputs {
            modes: &mut modes,
            options: &options,
        };
        let _ = tick_pending(
            &mut self.app,
            &mut self.state,
            self.runtime.as_ref(),
            &self.store,
            RenderFormat::Text,
            &mut self.session,
            &mut setup,
        );
    }
}

/// The handoff inputs the drain passes but never uses: these tests drain
/// `/resume` lines only, so neither terminal-mode half can ever run.
struct NoModes;

impl crate::interactive::tui::terminal_handoff::TerminalModes for NoModes {
    fn release(&mut self) -> std::io::Result<()> {
        Ok(())
    }

    fn reacquire(&mut self) -> std::io::Result<()> {
        Ok(())
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
/// (a) TYPED success: after a real `/sql` capture (and a planted gap), a
/// successful typed `/resume <id>` through the real `tick_pending` clears
/// the capture, the last query, AND the agent-capture gap.
#[test]
fn the_typed_resume_clears_the_previous_sessions_capture_last_query_and_gap() {
    let mut fx = Seed::new("typed-success", "cap-typed-live");
    fx.capture_marker_query();
    fx.app.agent_captures.gap = Some(CaptureGap::Missing);
    block_on(
        fx.store
            .save(SessionState::new("resumed-typed-ok", None, "test-model").redacted()),
    )
    .expect("the resumed record saves");
    fx.drain_pending("/resume resumed-typed-ok");
    assert!(
        fx.app
            .transcript
            .blocks()
            .iter()
            .any(|block| block.text.contains("Resumed session resumed-typed-ok")),
        "the typed resume said it resumed, not {:?}",
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
    assert!(
        fx.app.agent_captures.gap.is_none(),
        "the resumed session inherited the previous session's capture gap"
    );
    let root = fx.root.clone();
    drop(fx);
    wait_for_background_save(&root.join("store"));
    let _ = std::fs::remove_dir_all(&root);
}
/// (b) TYPED refusal: with the resumed id's lock held (the D4 technique),
/// the refused typed resume changes NOTHING — the capture, the last query,
/// and the gap all survive for the still-live session.
#[test]
fn the_refused_typed_resume_keeps_the_previous_sessions_query_state() {
    let mut fx = Seed::new("typed-refusal", "cap-typed-held-live");
    fx.capture_marker_query();
    fx.app.agent_captures.gap = Some(CaptureGap::Missing);
    let blocker = SessionRuntime::acquire(
        &fx.runtime,
        None,
        false,
        None,
        "resumed-typed-held",
        ApprovalPolicy::Ask,
        &fx.root.join("session-dirs"),
    )
    .expect("the blocker session acquires");
    block_on(
        fx.store
            .save(SessionState::new("resumed-typed-held", None, "test-model").redacted()),
    )
    .expect("the resumed record saves");
    fx.drain_pending("/resume resumed-typed-held");
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
    assert!(
        matches!(fx.app.agent_captures.gap, Some(CaptureGap::Missing)),
        "the refused resume kept the gap"
    );
    let root = fx.root.clone();
    drop(blocker);
    drop(fx);
    wait_for_background_save(&root.join("store"));
    let _ = std::fs::remove_dir_all(&root);
}
/// (c) TYPED success adopts the resumed session's runtime metrics collector:
/// `App.session_metrics` must be the NEW collector `session.session_metrics()`
/// now hands out (`reacquire` replaced the runtime). The enable flag is
/// flipped exactly as local_metrics_tests does; the initial acquire runs
/// while it is off, so the pre-fix app handle (None) cannot vacuously match.
#[test]
fn the_typed_resume_adopts_the_resumed_sessions_metrics_collector() {
    let mut fx = Seed::new("typed-metrics", "cap-typed-metrics-live");
    block_on(
        fx.store
            .save(SessionState::new("resumed-metrics", None, "test-model").redacted()),
    )
    .expect("the resumed record saves");
    set_enabled(true);
    fx.drain_pending("/resume resumed-metrics");
    let adopted = matches!(
        (&fx.app.session_metrics, &fx.session.session_metrics()),
        (Some(app_handle), Some(new_handle)) if Arc::ptr_eq(app_handle, new_handle)
    );
    set_enabled(false);
    assert!(
        adopted,
        "the app kept the previous session's metrics collector (or lost it)"
    );
    let root = fx.root.clone();
    drop(fx);
    wait_for_background_save(&root.join("store"));
    let _ = std::fs::remove_dir_all(&root);
}
