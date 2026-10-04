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
use crate::interactive::session_runtime::SessionRuntime;
use crate::interactive::session_state::SessionState;
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
            let cancellation = CancellationToken::new();
            fx.app.sql_task = Some((
                sql_task::spawn(
                    permit,
                    Arc::clone(&fx.runtime),
                    task.clone(),
                    cancellation.clone(),
                ),
                task,
                started,
                cancellation,
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

/// The headless SQL boundary and the real TUI worker execute the same bounded
/// operation against the same SQLite database and produce the same rows.
#[test]
fn direct_sql_operation_matches_the_tui_worker_result() {
    let mut fx = SaveRoundTripFixture::build();
    let sql = "SELECT * FROM t";
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime builds");
    let plain_result = runtime
        .block_on(sql_operation::execute(
            &fx.runtime,
            Some("analytics"),
            sql,
            false,
        ))
        .expect("the shared SQL operation succeeds");

    assert!(matches!(
        fx.dispatch_line("/connect analytics"),
        Dispatch::Handled
    ));
    let Dispatch::SqlTask(task) = fx.dispatch_line(&format!("/sql {sql}")) else {
        panic!("/sql dispatches its worker task");
    };
    let _pool = test_permit_lock();
    match fx.app.admit_second_sql() {
        SecondSqlDecision::Start(permit) => {
            let started = Instant::now();
            let cancellation = CancellationToken::new();
            fx.app.sql_task = Some((
                sql_task::spawn(
                    permit,
                    Arc::clone(&fx.runtime),
                    task.clone(),
                    cancellation.clone(),
                ),
                task,
                started,
                cancellation,
            ));
            fx.app.request.started = Some(started);
            fx.app.request.activity = Some("query".into());
        }
        SecondSqlDecision::Reject(message) => panic!("admission refused the query: {message}"),
    }
    tick_until(
        &mut fx,
        |fx| fx.app.captured.is_some(),
        "the /sql worker never completed into the capture",
    );

    let tui_result = &fx
        .app
        .captured
        .as_ref()
        .expect("successful query is captured")
        .result;
    assert_eq!(plain_result.columns, tui_result.columns);
    assert_eq!(plain_result.rows, tui_result.rows);
    assert_eq!(plain_result.row_count, tui_result.row_count);
    assert_eq!(plain_result.truncated, tui_result.truncated);
    assert_eq!(
        plain_result.row_count, 1,
        "the configured row cap is applied"
    );
    assert!(
        plain_result.truncated,
        "the cap reports more rows were available"
    );
}

#[test]
fn direct_sql_operation_preserves_profile_and_read_only_errors() {
    let fx = SaveRoundTripFixture::build();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime builds");

    let no_profile = runtime
        .block_on(sql_operation::execute(&fx.runtime, None, "SELECT 1", false))
        .expect_err("a missing active profile is refused");
    assert_eq!(
        no_profile.to_string(),
        "No active profile. Use /connect <profile> first."
    );
    let no_profile_event = runtime.block_on(crate::interactive::tui::exec::run_sql(
        &fx.runtime,
        None,
        "SELECT 1",
        &CancellationToken::new(),
        &std::sync::Arc::new(std::sync::Mutex::new(None)),
    ));
    assert!(matches!(
        no_profile_event,
        crate::render::TerminalEvent::Error { message }
            if message == no_profile.to_string()
    ));
    let invalid_profile = runtime
        .block_on(sql_operation::execute(
            &fx.runtime,
            Some("missing"),
            "SELECT 1",
            false,
        ))
        .expect_err("an unknown profile is refused");
    assert!(invalid_profile.to_string().contains("not found"));
    let invalid_event = runtime.block_on(crate::interactive::tui::exec::run_sql(
        &fx.runtime,
        Some("missing"),
        "SELECT 1",
        &CancellationToken::new(),
        &std::sync::Arc::new(std::sync::Mutex::new(None)),
    ));
    assert!(matches!(
        invalid_event,
        crate::render::TerminalEvent::Error { message }
            if message == invalid_profile.to_string()
    ));

    let sql = "DELETE FROM t";
    let operation_error = runtime
        .block_on(sql_operation::execute(
            &fx.runtime,
            Some("analytics"),
            sql,
            false,
        ))
        .expect_err("direct SQL cannot write");
    let event = runtime.block_on(crate::interactive::tui::exec::run_sql(
        &fx.runtime,
        Some("analytics"),
        sql,
        &CancellationToken::new(),
        &std::sync::Arc::new(std::sync::Mutex::new(None)),
    ));
    let crate::render::TerminalEvent::Error { message } = event else {
        panic!("an unsafe query is rendered as an error");
    };
    assert_eq!(message, operation_error.to_string());
}

#[test]
fn a_pre_cancelled_sql_operation_stops_before_observable_connector_build() {
    let mut fx = SaveRoundTripFixture::build();
    Arc::make_mut(&mut fx.runtime).connections.profiles.insert(
        "unopened".into(),
        DatabaseProfile::Sqlite {
            path: ":memory:".into(),
            read_only: true,
        },
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime builds");

    let uncancelled = runtime
        .block_on(sql_operation::execute(
            &fx.runtime,
            Some("unopened"),
            "SELECT 1",
            false,
        ))
        .expect_err("the synthetic in-memory profile reaches connector build");
    assert!(
        uncancelled
            .to_string()
            .contains("SQLite :memory: is not supported"),
        "the control observes the connector build boundary: {uncancelled}"
    );

    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let cancelled = runtime
        .block_on(sql_operation::execute_with_cancellation_and_source(
            &fx.runtime,
            Some("unopened"),
            "SELECT 1",
            false,
            &cancellation,
        ))
        .expect_err("a pre-cancelled operation stops before connector setup");

    assert_eq!(cancelled.to_string(), "query cancelled");
}

#[test]
fn direct_sql_operation_uses_the_configured_query_timeout() {
    const SLOW_QUERY: &str = "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM cnt \
        WHERE x < 500000000) SELECT count(*) FROM cnt";
    let mut fx = SaveRoundTripFixture::build();
    std::sync::Arc::make_mut(&mut fx.runtime)
        .resolved
        .query_timeout_seconds = 1;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime builds");
    let error = runtime
        .block_on(sql_operation::execute(
            &fx.runtime,
            Some("analytics"),
            SLOW_QUERY,
            false,
        ))
        .expect_err("the configured one-second deadline interrupts the query");
    assert!(
        error.to_string().to_lowercase().contains("timed out"),
        "the operation preserves timeout errors: {error}"
    );
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

/// Creates the five-row table the envelope tests query: five rows through a
/// cap of two must report two rows AND truncation, so the asserted cap is
/// the one the query actually used. A dedicated table keeps each test's SQL
/// text unique — the worker's envelope stash is keyed by (profile, sql).
fn seed_five_rows(path: &Path) {
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
        sqlx::query("CREATE TABLE t5 (id INTEGER PRIMARY KEY)")
            .execute(&pool)
            .await
            .expect("seed table");
        for id in 1..=5 {
            sqlx::query("INSERT INTO t5 (id) VALUES (?)")
                .bind(id)
                .execute(&pool)
                .await
                .expect("seed row");
        }
        pool.close().await;
    });
}

fn db_path(fx: &SaveRoundTripFixture) -> PathBuf {
    fx.root.0.join("capture.sqlite3")
}

/// Admits and spawns a dispatched task exactly as `loop_tick::pending` does.
/// The caller holds the permit-pool guard across the worker's life.
fn spawn_real_sql_task(
    fx: &mut SaveRoundTripFixture,
    task: sql_task::SqlTask,
    _pool: &tokio::sync::MutexGuard<'static, ()>,
) {
    match fx.app.admit_second_sql() {
        SecondSqlDecision::Start(permit) => {
            let started = Instant::now();
            let cancellation = CancellationToken::new();
            fx.app.sql_task = Some((
                sql_task::spawn(
                    permit,
                    Arc::clone(&fx.runtime),
                    task.clone(),
                    cancellation.clone(),
                ),
                task,
                started,
                cancellation,
            ));
            fx.app.request.started = Some(started);
            fx.app.request.activity = Some("query".into());
        }
        SecondSqlDecision::Reject(message) => panic!("admission refused the query: {message}"),
    }
}

/// Replaces the config behind BOTH runtime handles (the fixture's and the
/// app's) so the worker and the completion poll see the same runtime.
fn set_max_rows(fx: &mut SaveRoundTripFixture, max_rows: usize) {
    let mut config = (*fx.runtime).clone();
    config.resolved.max_rows = max_rows;
    let fresh = Arc::new(config);
    fx.runtime = Arc::clone(&fresh);
    fx.app.runtime = fresh;
}

/// RED-first (E2-2): the cancellation-aware source variant does not exist
/// yet — this fails to compile until `execute_with_cancellation_and_source`
/// lands next to `execute_resolved_with_source`.
#[test]
fn cancellation_aware_source_variant_mirrors_the_plain_envelope() {
    let _pool = test_permit_lock();
    let mut fx = SaveRoundTripFixture::build();
    seed_five_rows(&db_path(&fx));
    set_max_rows(&mut fx, 2);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime builds");

    let execution = runtime
        .block_on(sql_operation::execute_with_cancellation_and_source(
            &fx.runtime,
            Some("analytics"),
            "SELECT * FROM t5 ORDER BY id",
            false,
            &CancellationToken::new(),
        ))
        .expect("the cancellation-aware source variant succeeds");
    let profile = fx
        .runtime
        .named_profile("analytics")
        .expect("fixture profile");
    let expected_identity = profile_identity("analytics", profile, &fx.runtime.cache_scope);
    assert_eq!(execution.binding.profile_name, "analytics");
    assert_eq!(
        execution.binding.profile_identity,
        expected_identity.as_str(),
        "the variant binds the SAME opaque profile identity",
    );
    assert_eq!(execution.binding.dialect, SqlDialect::Sqlite);
    assert_eq!(
        execution.binding.applied_row_cap, 2,
        "the variant binds the cap the query actually used",
    );
    assert_eq!(execution.result.rows.len(), 2);
    assert!(execution.result.truncated);
    assert!(execution.started_unix_ms <= execution.finished_unix_ms);

    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let stopped = runtime
        .block_on(sql_operation::execute_with_cancellation_and_source(
            &fx.runtime,
            Some("analytics"),
            "SELECT * FROM t5 ORDER BY id",
            false,
            &cancelled,
        ))
        .expect_err("a pre-cancelled variant stops before connector setup");
    assert_eq!(stopped.to_string(), "query cancelled");
}

/// Proof (a): after a direct SQL task completes through the REAL worker, the
/// captured evidence names the EXECUTED source — opaque identity (not None)
/// and the cap the query actually used — with nondecreasing timestamps.
#[test]
fn tui_worker_evidence_comes_from_the_executed_envelope() {
    let _pool = test_permit_lock();
    let mut fx = SaveRoundTripFixture::build();
    seed_five_rows(&db_path(&fx));
    set_max_rows(&mut fx, 2);
    let profile = fx
        .runtime
        .named_profile("analytics")
        .expect("fixture profile");
    let expected_identity = profile_identity("analytics", profile, &fx.runtime.cache_scope)
        .as_str()
        .to_owned();

    assert!(matches!(
        fx.dispatch_line("/connect analytics"),
        Dispatch::Handled
    ));
    let Dispatch::SqlTask(task) = fx.dispatch_line("/sql SELECT * FROM t5 ORDER BY id") else {
        panic!("/sql dispatches its worker task");
    };
    spawn_real_sql_task(&mut fx, task, &_pool);
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
    assert_eq!(capture.result.rows.len(), 2);
    assert!(
        capture.result.truncated,
        "five rows through a cap of two report truncation",
    );
    let evidence = &capture.evidence;
    assert_eq!(
        evidence.connection_identity.as_deref(),
        Some(expected_identity.as_str()),
        "the evidence names the executed source identity, never None",
    );
    assert_eq!(
        evidence.max_rows, 2,
        "the evidence names the cap the query actually used",
    );
    assert_eq!(evidence.connection_label, "analytics");
    assert_eq!(evidence.dialect, SqlDialect::Sqlite);
    assert_eq!(evidence.source, EvidenceSource::DirectSql);
    assert!(
        evidence.started_unix_ms <= evidence.finished_unix_ms,
        "the envelope timestamps never run backwards",
    );
}

/// Proof (b) IMMUTABILITY: after spawning the task but BEFORE its completion
/// is consumed, the live runtime is mutated (row cap 2 -> 500, profile
/// endpoint repointed to a new identity) — the consumed evidence must still
/// describe the EXECUTED source.
#[test]
fn tui_worker_evidence_survives_a_runtime_mutation_before_consumption() {
    let _pool = test_permit_lock();
    let mut fx = SaveRoundTripFixture::build();
    seed_five_rows(&db_path(&fx));
    set_max_rows(&mut fx, 2);
    let profile = fx
        .runtime
        .named_profile("analytics")
        .expect("fixture profile");
    let expected_identity = profile_identity("analytics", profile, &fx.runtime.cache_scope)
        .as_str()
        .to_owned();

    assert!(matches!(
        fx.dispatch_line("/connect analytics"),
        Dispatch::Handled
    ));
    let Dispatch::SqlTask(task) = fx.dispatch_line("/sql SELECT id FROM t5 ORDER BY id") else {
        panic!("/sql dispatches its worker task");
    };
    spawn_real_sql_task(&mut fx, task, &_pool);

    // Mutate ONLY what the completion sees: the worker already owns its
    // runtime Arc, so this changes the consumption-time runtime alone.
    set_max_rows(&mut fx, 500);
    {
        let mut config = (*fx.runtime).clone();
        config.connections.profiles.insert(
            "analytics".to_string(),
            DatabaseProfile::Sqlite {
                path: "/nonexistent-mutated.sqlite3".to_string(),
                read_only: true,
            },
        );
        let fresh = Arc::new(config);
        fx.runtime = Arc::clone(&fresh);
        fx.app.runtime = fresh;
    }
    let mutated_identity = {
        let profile = fx
            .runtime
            .named_profile("analytics")
            .expect("mutated profile");
        profile_identity("analytics", profile, &fx.runtime.cache_scope)
            .as_str()
            .to_owned()
    };
    assert_ne!(
        mutated_identity, expected_identity,
        "the mutation really repoints the source identity",
    );

    tick_until(
        &mut fx,
        |fx| fx.app.captured.is_some(),
        "the /sql worker never completed into the capture",
    );
    let evidence = &fx
        .app
        .captured
        .as_ref()
        .expect("a successful /sql is captured")
        .evidence;
    assert_eq!(
        evidence.max_rows, 2,
        "the evidence keeps the EXECUTED cap, not the mutated 500",
    );
    assert_eq!(
        evidence.connection_identity.as_deref(),
        Some(expected_identity.as_str()),
        "the evidence keeps the EXECUTED identity, not the mutated endpoint",
    );
}

/// Proof (c): a read-only-rejected query through the REAL worker keeps the
/// old error behavior and captures nothing.
#[test]
fn tui_worker_read_only_rejection_keeps_error_behavior_and_captures_nothing() {
    use crate::interactive::tui::transcript::BlockKind;

    let _pool = test_permit_lock();
    let mut fx = SaveRoundTripFixture::build();
    set_max_rows(&mut fx, 2);
    assert!(matches!(
        fx.dispatch_line("/connect analytics"),
        Dispatch::Handled
    ));
    let sql = "DELETE FROM t";
    let Dispatch::SqlTask(task) = fx.dispatch_line(&format!("/sql {sql}")) else {
        panic!("/sql dispatches its worker task");
    };
    spawn_real_sql_task(&mut fx, task, &_pool);
    tick_until(
        &mut fx,
        |fx| fx.app.sql_task.is_none(),
        "the rejected worker never settled",
    );

    assert!(
        fx.app.captured.is_none(),
        "a rejected query captures nothing",
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime builds");
    let expected = runtime
        .block_on(sql_operation::execute(
            &fx.runtime,
            Some("analytics"),
            sql,
            false,
        ))
        .expect_err("direct SQL cannot write");
    let rendered = fx
        .app
        .transcript
        .blocks()
        .iter()
        .rev()
        .find(|block| block.kind == BlockKind::Error)
        .map(|block| block.text.clone())
        .expect("the rejection renders as an error block");
    assert_eq!(rendered, expected.to_string());
}
