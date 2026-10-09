//! E2-4 A1/A3: through the REAL `bounded_sql_query` tool path against REAL
//! SQLite, the capture that reaches the TUI builds `ExecutionEvidence` with
//! the connection's real profile identity, the Sqlite dialect, the model's
//! small clipped row cap, and an Agent source — and the same alias resolving
//! to a different sqlite path yields a different connection identity.

use super::{CaptureEvent, CaptureHook};
use crate::agent::tools::DatabaseTools;
use crate::connection::{ConnectionEntry, ConnectionRegistry};
use crate::interactive::tui::capture::agent_evidence;
use saya_agent::ToolExecutor;
use saya_connectors::{ConnectorOptions, DatabaseConnector, SqliteConnector};
use saya_types::{EvidenceSource, ResultScope, SqlDialect};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

static E24_FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

type Observed = Arc<Mutex<Vec<CaptureEvent>>>;

fn observing_hook() -> (CaptureHook, Observed) {
    let observed: Observed = Arc::new(Mutex::new(Vec::new()));
    let sink = observed.clone();
    let hook: CaptureHook = Arc::new(move |event| sink.lock().unwrap().push(event));
    (hook, observed)
}

fn loop_budget() -> usize {
    saya_agent::AgentLimits::default().context_byte_budget
}

/// The model row cap this test configures via `with_registry`: the oracle
/// every cap-side assertion checks against, never read back from a capture.
const MODEL_ROW_CAP: usize = 2;

/// A seeded SQLite file plus a read-only connector over it, mirroring the
/// neighbor fixture (seed with a plain pool, close it, open read-only).
async fn e24_sqlite_fixture(rows: usize) -> (Box<dyn DatabaseConnector>, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "saya-e24-receipt-{}-{}",
        std::process::id(),
        E24_FIXTURE_ID.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("receipt.db");
    let seed = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&db)
            .create_if_missing(true),
    )
    .await
    .unwrap();
    sqlx::query("CREATE TABLE items (id INTEGER)")
        .execute(&seed)
        .await
        .unwrap();
    let values = (0..rows)
        .map(|id| format!("({id})"))
        .collect::<Vec<_>>()
        .join(", ");
    sqlx::query(&format!("INSERT INTO items VALUES {values}"))
        .execute(&seed)
        .await
        .unwrap();
    seed.close().await;
    let connector = SqliteConnector::open(&db, true, ConnectorOptions::default())
        .await
        .unwrap();
    (Box::new(connector), dir)
}

/// A read-only connector over a seeded sqlite file: `path` selects the
/// physical source, so the same alias over two paths resolves two entries.
async fn e24_connector(
    dir: &std::path::Path,
    file: &str,
    rows: usize,
) -> Box<dyn DatabaseConnector> {
    let db = dir.join(file);
    let seed = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&db)
            .create_if_missing(true),
    )
    .await
    .expect("seed E2-4 sqlite database");
    sqlx::query("CREATE TABLE items (id INTEGER)")
        .execute(&seed)
        .await
        .expect("create E2-4 table");
    let values = (0..rows)
        .map(|id| format!("({id})"))
        .collect::<Vec<_>>()
        .join(", ");
    sqlx::query(&format!("INSERT INTO items VALUES {values}"))
        .execute(&seed)
        .await
        .expect("seed E2-4 rows");
    seed.close().await;
    Box::new(
        SqliteConnector::open(&db, true, ConnectorOptions::default())
            .await
            .expect("open E2-4 sqlite connector"),
    )
}

fn e24_root(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "saya-e24-{tag}-{}-{}",
        std::process::id(),
        E24_FIXTURE_ID.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A1: a 5-row table under a model-row-cap-scale `max_rows` clips the model
/// view; the captured result's evidence carries the REAL profile identity
/// (copied from the registry entry, like the resolver hands the tool), the
/// Sqlite dialect, the tool's applied cap, Agent source, ModelLimited scope,
/// consistent counts, and nondecreasing timestamps.
#[tokio::test]
async fn agent_path_evidence_carries_real_identity_and_clipped_cap() {
    let (_connector, _keep_alive) = e24_sqlite_fixture(60).await;
    let dir = e24_root("a1");
    let identity = "e24-real-profile-identity";
    let mut registry = ConnectionRegistry::new("analytics");
    registry.insert(
        "analytics",
        ConnectionEntry {
            connector: e24_connector(&dir, "a1.sqlite3", 5).await,
            dialect: SqlDialect::Sqlite,
            profile_id: Some(identity.to_string()),
        },
    );
    let expected_identity = registry
        .resolve(Some("analytics"))
        .expect("analytics resolves")
        .profile_id
        .clone();
    let (hook, observed) = observing_hook();
    let tools = DatabaseTools::with_registry(registry, MODEL_ROW_CAP, true, None)
        .with_capture(Some(hook), loop_budget());
    let _value = tools
        .execute(
            "bounded_sql_query",
            serde_json::json!({"sql": "SELECT id FROM items ORDER BY id"}),
        )
        .await
        .expect("real sqlite agent query succeeds");
    let events = observed.lock().unwrap();
    assert_eq!(events.len(), 1, "exactly one capture event: {events:?}");
    let CaptureEvent::Captured(capture) = &events[0] else {
        panic!("a small clean result is captured, got {:?}", events[0]);
    };
    let evidence = agent_evidence(capture);
    assert_eq!(
        evidence.connection_identity, expected_identity,
        "identity comes from the real registry entry, not the test string"
    );
    assert_eq!(
        expected_identity,
        Some(identity.to_string()),
        "the real entry carries the planted identity"
    );
    assert_eq!(evidence.dialect, SqlDialect::Sqlite);
    assert_eq!(
        capture.row_cap, MODEL_ROW_CAP,
        "the tool applied the configured model cap"
    );
    assert_eq!(
        evidence.max_rows, MODEL_ROW_CAP,
        "max_rows is the cap the tool applied"
    );
    assert_eq!(evidence.source, EvidenceSource::Agent);
    assert_eq!(
        evidence.scope,
        ResultScope::ModelLimited {
            row_cap: MODEL_ROW_CAP
        },
    );
    assert_eq!(
        capture.result.rows.len(),
        MODEL_ROW_CAP,
        "the 5-row table is clipped to the configured model cap"
    );
    assert!(capture.result.truncated, "the clip marks truncation");
    assert_eq!(evidence.returned_rows, MODEL_ROW_CAP);
    assert!(
        evidence.truncated,
        "the evidence says the physical result was cut"
    );
    assert!(
        evidence.knowledge_ids.is_empty(),
        "no knowledge is claimed applied/verified: the agent path never \
         supplies knowledge ids, so the evidence marks none"
    );
    assert!(
        capture.started_unix_ms <= capture.finished_unix_ms,
        "timestamps never run backwards"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[path = "capture_receipt_tests/alias_identity.rs"]
mod alias_identity;
