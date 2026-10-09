//! The real save boundary for captured `/sql` rows: a capture driven through
//! the app's own dispatch and worker path — real SQLite query, real
//! `admit_second_sql` admission, real `tick_workers` poll — must never reach
//! the session file `queue_session_save` writes. The sentinel is a queried
//! ROW VALUE (a one-cell table), never part of the input SQL: the command
//! and its history are intentionally persisted conversation data, the
//! result rows are not.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use saya_agent::{ApprovalPolicy, CancellationToken};
use saya_store::FsSessionStore;
use saya_types::DatabaseProfile;

use crate::config::runtime::RuntimeConfig;
use crate::interactive::session::runtime::SessionRuntime;
use crate::interactive::session::state::SessionState;
use crate::interactive::sql_operation;
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
static FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

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
        sqlx::query("INSERT INTO t (value) VALUES ('second-row')")
            .execute(&pool)
            .await
            .expect("seed second row");
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
        Self::build_with_timeout(60)
    }

    fn build_with_timeout(query_timeout_seconds: u64) -> Self {
        let root = std::env::temp_dir().join(format!(
            "saya-capture-save-{SESSION_ID}-{}-{}",
            std::process::id(),
            FIXTURE_ID.fetch_add(1, Ordering::Relaxed),
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
        config.resolved.max_rows = 1;
        config.resolved.query_timeout_seconds = query_timeout_seconds;
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
#[cfg(test)]
#[path = "capture_save_sql_cancellation_tests.rs"]
mod sql_cancellation_tests;

// E2-2: the TUI direct-SQL evidence must describe the EXECUTED source — the
// envelope the worker's query ran with — not whatever the mutable runtime
// says when the completion is consumed. These tests drive the REAL worker
// (dispatch, admit, spawn, tick) against real SQLite, then read the capture.
use crate::profile_identity::profile_identity;
use saya_types::{EvidenceSource, SqlDialect};
#[path = "capture_save_tests/helpers.rs"]
mod helpers;
use self::helpers::*;
#[path = "capture_save_tests/direct_sql.rs"]
mod direct_sql;
#[path = "capture_save_tests/envelope.rs"]
mod envelope;
#[path = "capture_save_tests/privacy.rs"]
mod privacy;
