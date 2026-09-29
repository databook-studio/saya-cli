//! The real save boundary for captured `/sql` rows: a capture driven through
//! the app's own dispatch and worker path — real SQLite query, real
//! `admit_second_sql` admission, real `tick_workers` poll — must never reach
//! the session file `queue_session_save` writes. The sentinel is a queried
//! ROW VALUE (a one-cell table), never part of the input SQL: the command
//! and its history are intentionally persisted conversation data, the
//! result rows are not.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use saya_agent::ApprovalPolicy;
use saya_store::FsSessionStore;
use saya_types::DatabaseProfile;

use crate::config::runtime::RuntimeConfig;
use crate::interactive::session_runtime::SessionRuntime;
use crate::interactive::session_state::SessionState;
use crate::interactive::tui::application::SecondSqlDecision;
use crate::interactive::tui::dispatch::{Dispatch, dispatch};
use crate::interactive::tui::loop_tick::workers::tick_workers;
use crate::interactive::tui::session_save::queue_session_save;
use crate::interactive::tui::sql_task;
use crate::interactive::tui::types::App;
use crate::interactive::tui::ui_snapshot_tests::{empty_app, unused_runtime};
use crate::interactive::tui::worker_permits::test_permit_lock;
use crate::render::RenderFormat;

const SESSION_ID: &str = "capture-save-test";

/// The row-only sentinel: unique per run, and never written into the input
/// SQL — only into the table cell the query returns.
fn sentinel() -> String {
    format!("ROW-ONLY-SENTINEL-{}", std::process::id())
}

/// Removes the throwaway root when the fixture drops. Declared after the
/// session runtime in the fixture so the lock file is released before the
/// directory goes.
struct RootGuard(PathBuf);

impl Drop for RootGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Creates the one-cell SQLite table the `/sql` task queries: the sentinel
/// is a row VALUE, never part of the input SQL.
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
            .expect("seed row");
        pool.close().await;
    });
}

/// One fixture with the exact pieces the real loop takes: `dispatch` (the
/// shape `loop_tick::pending` passes), the worker poll `tick_workers`, and
/// the real save boundary `queue_session_save` — all backed by a throwaway
/// root holding a one-cell SQLite database.
struct SaveRoundTripFixture {
    app: App,
    state: SessionState,
    session: SessionRuntime,
    store: FsSessionStore,
    runtime: Arc<RuntimeConfig>,
    root: RootGuard,
}

impl SaveRoundTripFixture {
    fn build() -> Self {
        let root = std::env::temp_dir().join(format!(
            "saya-capture-save-{SESSION_ID}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("save-round-trip test root");
        let db = root.join("capture.sqlite3");
        seed_sqlite(&db, &sentinel());
        let mut config = std::sync::Arc::unwrap_or_clone(unused_runtime());
        config.connections.profiles.insert(
            "analytics".to_string(),
            DatabaseProfile::Sqlite {
                path: db.display().to_string(),
                read_only: true,
            },
        );
        let runtime = Arc::new(config);
        let mut app = empty_app();
        app.runtime = Arc::clone(&runtime);
        let state = SessionState::new(SESSION_ID, None, "qwen");
        let session = SessionRuntime::acquire(
            &runtime,
            Some(&root),
            true,
            None,
            state.id.as_str(),
            ApprovalPolicy::Ask,
            &root.join("sessions"),
        )
        .expect("the save-round-trip session acquires");
        Self {
            app,
            state,
            session,
            store: FsSessionStore::new(root.join("session-store")),
            runtime,
            root: RootGuard(root),
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
}

/// Ticks the real worker poll until `ready` — the loop the TUI runs — with
/// a bounded deadline so a broken worker fails the test, never hangs it.
fn tick_until(
    fx: &mut SaveRoundTripFixture,
    ready: impl Fn(&SaveRoundTripFixture) -> bool,
    what: &str,
) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready(fx) {
        assert!(Instant::now() < deadline, "{what}");
        tick_workers(&mut fx.app, &fx.store, &mut fx.state);
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// The contract at its strongest boundary: after a real `/sql SELECT * FROM t`
/// completes into `App.captured` through the worker path, the real
/// `queue_session_save` write lands in the session dir without the captured
/// row value. The mutation proof (a captured-result copy into a persisted
/// turn, applied then reverted) turns the final assertion red.
#[test]
fn a_captured_sql_result_never_reaches_the_saved_session() {
    let mut fx = SaveRoundTripFixture::build();
    let sentinel = sentinel();

    // The real dispatch chain: select the profile, then run the query.
    let connected = fx.dispatch_line("/connect analytics");
    assert!(
        matches!(connected, Dispatch::Handled),
        "the connect is handled"
    );
    assert_eq!(
        fx.state.profile.as_deref(),
        Some("analytics"),
        "the profile is selected"
    );
    let outcome = fx.dispatch_line("/sql SELECT * FROM t");
    let Dispatch::SqlTask(task) = outcome else {
        panic!("/sql dispatches its worker task");
    };

    // The real admission + spawn, exactly as `loop_tick::pending` does.
    let _pool = test_permit_lock();
    match fx.app.admit_second_sql() {
        SecondSqlDecision::Start(permit) => {
            let started = Instant::now();
            fx.app.sql_task = Some((
                sql_task::spawn(permit, Arc::clone(&fx.runtime), task.clone()),
                task,
                started,
            ));
            fx.app.request.started = Some(started);
            fx.app.request.activity = Some("query".into());
        }
        SecondSqlDecision::Reject(message) => panic!("admission refused the query: {message}"),
    }

    // The real worker poll: the completion lands in the capture.
    tick_until(
        &mut fx,
        |fx| fx.app.captured.is_some(),
        "the /sql worker never completed into the capture",
    );
    let capture = fx
        .app
        .captured
        .as_ref()
        .expect("a successful /sql is captured");
    let cell = capture.result.rows[0].as_array().expect("a row array")[0]
        .as_str()
        .expect("a text cell");
    assert_eq!(cell, sentinel, "the capture holds the queried row value");
    assert!(
        !capture.result.executed_sql.contains(&sentinel),
        "the sentinel must be a row value, not part of the SQL: {}",
        capture.result.executed_sql
    );

    // The real save boundary, then the real poll until it lands.
    queue_session_save(&mut fx.app, &fx.store, &fx.state);
    tick_until(
        &mut fx,
        |fx| fx.app.session_save.is_none() && fx.app.pending_session_save.is_none(),
        "the session save never completed",
    );
    let saved = std::fs::read_to_string(
        fx.root
            .0
            .join("session-store")
            .join(format!("{SESSION_ID}.json")),
    )
    .expect("the queued save wrote the session file");
    assert!(
        saved.contains(SESSION_ID),
        "our session really was written: {saved}"
    );
    assert!(
        !saved.contains(&sentinel),
        "captured result rows reached the saved session: {saved}"
    );
}
