//! E2-4b P1/P2: a planted row value returned by a real `/sql` worker turn
//! never reaches the saved session record, and after save + load through the
//! real picker resume (`tick_resume` — `resume_session` + `reacquire`) the
//! loaded session offers NO last query, NO captured evidence, and NO prior
//! tool-outcome facts — receipts stay memory-only, never rebuilt. The
//! planted-row/saved-bytes half is separately pinned by
//! `a_captured_sql_result_never_reaches_the_saved_session`
//! (tui/capture_save_tests.rs:185) with its own sentinel.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use saya_agent::{ApprovalPolicy, CancellationToken};
use saya_store::FsSessionStore;
use saya_types::DatabaseProfile;

use crate::config::runtime::RuntimeConfig;
use crate::interactive::session_runtime::SessionRuntime;
use crate::interactive::session_state::SessionState;
use crate::interactive::tui::application::SecondSqlDecision;
use crate::interactive::tui::dispatch::{Dispatch, dispatch};
use crate::interactive::tui::loop_tick::resume::tick_resume;
use crate::interactive::tui::loop_tick::workers::tick_workers;
use crate::interactive::tui::session_save::queue_session_save;
use crate::interactive::tui::sql_task;
use crate::interactive::tui::types::App;
use crate::interactive::tui::ui_snapshot_tests::{empty_app, unused_runtime};
use crate::interactive::tui::worker_permits::test_permit_lock;
use crate::render::RenderFormat;

/// The planted row value: it exists ONLY inside one SQLite cell the query
/// returns — never in the SQL text, the conversation, or the record.
const MARKER: &str = "PLANTED-ROW-VALUE-7d1c";
const SESSION_ID: &str = "p2-privacy";

/// Creates the one-cell SQLite table the worker turn queries: the marker is
/// a row VALUE, never part of the input SQL.
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

/// One round trip with the exact pieces the real loop takes.
struct RoundTrip {
    app: App,
    state: SessionState,
    session: SessionRuntime,
    runtime: Arc<RuntimeConfig>,
    store: FsSessionStore,
}

impl RoundTrip {
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

    /// Ticks the real worker poll until `ready` — the loop the TUI runs —
    /// with a bounded deadline so a broken worker fails, never hangs.
    fn tick(&mut self, what: &str, ready: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready(&self.app) {
            assert!(Instant::now() < deadline, "{what}");
            tick_workers(&mut self.app, &self.store, &mut self.state);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

#[test]
fn a_saved_session_resumes_with_no_receipt_facts_and_no_planted_row_value() {
    // --- the old runtime: a real /sql worker turn returns the planted row.
    let root = std::env::temp_dir().join(format!("saya-p2-privacy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("the privacy round-trip root");
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
    let store = FsSessionStore::new(root.join("session-store"));
    let mut fx = RoundTrip {
        app: empty_app(),
        state: SessionState::new(SESSION_ID, None, "test-model"),
        session: SessionRuntime::acquire(
            &runtime,
            Some(&root),
            true,
            None,
            SESSION_ID,
            ApprovalPolicy::Ask,
            &root.join("session-dirs"),
        )
        .expect("the old-runtime session acquires"),
        runtime,
        store,
    };
    fx.app.runtime = Arc::clone(&fx.runtime);
    assert!(
        matches!(fx.dispatch_line("/connect analytics"), Dispatch::Handled),
        "the profile connects"
    );
    let Dispatch::SqlTask(task) = fx.dispatch_line("/sql SELECT * FROM t") else {
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
        }
        SecondSqlDecision::Reject(message) => panic!("admission refused the query: {message}"),
    }
    fx.tick("the /sql worker never completed", |app| {
        app.captured.is_some()
    });
    let capture = fx
        .app
        .captured
        .as_ref()
        .expect("a successful /sql is captured");
    assert_eq!(
        capture.result.rows[0][0], MARKER,
        "the capture holds the planted row value"
    );

    // The real save boundary, then the real poll until it lands.
    queue_session_save(&mut fx.app, &fx.store, &fx.state);
    fx.tick("the session save never completed", |app| {
        app.session_save.is_none() && app.pending_session_save.is_none()
    });
    let saved_path = root
        .join("session-store")
        .join(format!("{SESSION_ID}.json"));
    let saved = std::fs::read_to_string(saved_path).expect("the queued save wrote the file");
    assert!(saved.contains(SESSION_ID), "our session really was written");
    assert!(
        !saved.contains(MARKER),
        "the planted row value reached the saved record: {saved}"
    );

    // --- the new runtime: a fresh process, a fresh App, the real picker
    // resume (`tick_resume` runs `resume_session` + `reacquire`). No profile
    // is configured here, so nothing of the current runtime could even name
    // the old query — the loaded session must offer no receipt facts.
    let runtime2 = unused_runtime();
    let mut app2 = empty_app();
    let mut state2 = SessionState::new("p2-current", None, "test-model");
    let mut session2 = SessionRuntime::acquire(
        &runtime2,
        Some(&root),
        true,
        None,
        "p2-current",
        ApprovalPolicy::Ask,
        &root.join("session-dirs-2"),
    )
    .expect("the new-runtime session acquires");
    app2.overlays.pending_resume = Some(SESSION_ID.into());
    tick_resume(
        &mut app2,
        &fx.store,
        &mut state2,
        runtime2.as_ref(),
        &mut session2,
    );

    // The resume is real: the saved session's id was adopted...
    assert_eq!(state2.id, SESSION_ID);
    // ...and every receipt fact stays unknown: memory-only, never rebuilt.
    assert!(
        app2.captured.is_none(),
        "the resumed session offers captured evidence"
    );
    assert!(
        app2.last_query.is_none(),
        "the resumed session offers a last query"
    );
    assert!(
        state2.prior_tool_outcomes.is_none(),
        "the loaded state carries prior tool outcomes"
    );
    let record = serde_json::to_string(&state2.redacted()).expect("the record serializes");
    assert!(
        !record.contains(MARKER),
        "the planted row value rides the loaded state: {record}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
