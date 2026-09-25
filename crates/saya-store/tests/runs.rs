//! The run-records store — contract and privacy tests for migration step 7.
//!
//! Three guarantees:
//! 1. The API is payload-free: no parameter accepts goal or plan text — a
//!    goal carrying a sentinel does not compile into any call — and the byte
//!    scan below proves the flow writes no sentinel into the SQLite file or
//!    its `-wal`/`-shm` sidecars. If a payload parameter is ever added, the
//!    schema allowlist test makes the new column a conscious, reviewable
//!    change and the scan turns red the moment it is written.
//! 2. Usage is nullable end to end: an unreported figure is NULL in the row
//!    and `None` in the API — "unknown", never zero — so a provider that
//!    reports nothing is never presented as having cost nothing.
//! 3. The schema stays metadata-only: every column of `runs` and `run_steps`
//!    is on an explicit allowlist, so a payload column cannot appear without
//!    a reviewer consciously adding it here.

use std::collections::BTreeMap;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use saya_store::{
    NewRun, RunBudgets, RunCapabilityFlags, RunStatus, RunStepStatus, RunStore, RunUsage,
    SchemaStore, SqliteStateStore, StoreError, state_sidecar_path,
};
use saya_types::{RunFailureCode, RunId};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

/// A goal carrying credential-shaped sentinels. It exists only to prove it
/// cannot be stored: the runs API has no goal parameter, and the byte scan
/// below would fail if any write path ever grew one.
const GOAL_SENTINELS: &[&str] = &[
    "api_key=sk-live-SENTINELGOALSECRET",
    "postgres://user:SENTINELPASSWORD@host/db",
];

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("saya-runs-{label}-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    root
}

fn run_id(tag: &str) -> RunId {
    RunId::parse(&format!("run-{tag}")).unwrap()
}

fn flags() -> RunCapabilityFlags {
    RunCapabilityFlags {
        workspace_write: true,
        fetch: false,
        runner: false,
        scratch: false,
    }
}

fn budgets() -> RunBudgets {
    RunBudgets {
        wall_clock_ms: Some(600_000),
        tokens_per_endpoint: BTreeMap::from([("primary".to_owned(), 100_000)]),
        turns: Some(50),
        tool_calls: Some(200),
        downloaded_bytes: None,
        workspace_bytes: Some(1 << 20),
        workspace_files: Some(100),
        process_count: None,
        process_time_ms: None,
    }
}

/// Usage with known figures for the `primary` endpoint.
fn usage_known() -> RunUsage {
    RunUsage {
        wall_clock_ms: Some(1_500),
        tokens_per_endpoint: BTreeMap::from([("primary".to_owned(), Some(500))]),
        turns: Some(3),
        tool_calls: Some(7),
    }
}

/// Usage where every figure is unreported: unknown, not zero.
fn usage_unknown() -> RunUsage {
    RunUsage {
        wall_clock_ms: None,
        tokens_per_endpoint: BTreeMap::from([("primary".to_owned(), None)]),
        turns: None,
        tool_calls: None,
    }
}

/// Concatenate the raw bytes of the database and any populated sidecars, the
/// `knowledge_security.rs` recipe: a freshly written row may live only in the
/// `-wal` file, so scanning all three is what makes the check honest.
fn db_bytes(db: &Path) -> Vec<u8> {
    let mut out = fs::read(db).unwrap_or_default();
    for suffix in ["-wal", "-shm"] {
        let sidecar = state_sidecar_path(db, suffix);
        if sidecar.exists() {
            out.extend(fs::read(&sidecar).unwrap_or_default());
        }
    }
    out
}

fn assert_absent(label: &str, bytes: &[u8], needle: &str) {
    assert!(
        !bytes
            .windows(needle.len())
            .any(|window| window == needle.as_bytes()),
        "LEAK: `{needle}` found in {label} bytes"
    );
}

async fn read_pool(db: &Path) -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(db))
        .await
        .unwrap()
}

// ---------------------------------------------------------------------------
// Creation and reads
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_run_records_metadata_and_reads_back() {
    let root = temp_root("create");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let record = store
        .create_run(NewRun {
            id: run_id("create"),
            capabilities: flags(),
            budgets: budgets(),
        })
        .await
        .unwrap();
    assert_eq!(record.status, RunStatus::Planned);
    assert_eq!(record.failure_code, None);
    assert_eq!(record.capabilities, flags());
    assert_eq!(record.budgets, budgets());
    // A fresh run has no usage at all: unknown, never zero.
    assert_eq!(record.usage, RunUsage::unknown());
    let read = store.get_run(&run_id("create")).await.unwrap().unwrap();
    assert_eq!(read.status, RunStatus::Planned);
    assert_eq!(read.budgets.tokens_per_endpoint["primary"], 100_000);
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn creating_the_same_run_twice_conflicts() {
    let root = temp_root("dup");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let run = NewRun {
        id: run_id("dup"),
        capabilities: flags(),
        budgets: budgets(),
    };
    store.create_run(run.clone()).await.unwrap();
    let error = store.create_run(run).await.unwrap_err();
    assert_eq!(error, StoreError::Conflict);
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn list_runs_returns_recent_first_summaries() {
    let root = temp_root("list");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    for tag in ["a", "b"] {
        store
            .create_run(NewRun {
                id: run_id(tag),
                capabilities: flags(),
                budgets: budgets(),
            })
            .await
            .unwrap();
        store
            .set_run_status(&run_id(tag), RunStatus::Approved, None)
            .await
            .unwrap();
    }
    let runs = store.list_runs().await.unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].id, run_id("b"));
    assert_eq!(runs[0].status, RunStatus::Approved);
    assert_eq!(runs[1].id, run_id("a"));
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Status transitions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn run_lifecycle_walks_the_state_machine() {
    let root = temp_root("lifecycle");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let id = run_id("lifecycle");
    store
        .create_run(NewRun {
            id: id.clone(),
            capabilities: flags(),
            budgets: budgets(),
        })
        .await
        .unwrap();
    for status in [
        RunStatus::Approved,
        RunStatus::Executing,
        RunStatus::Paused,
        RunStatus::Executing,
        RunStatus::Completed,
    ] {
        store.set_run_status(&id, status, None).await.unwrap();
        let record = store.get_run(&id).await.unwrap().unwrap();
        assert_eq!(
            record.status, status,
            "transition to {status:?} did not hold"
        );
    }
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn illegal_transitions_refuse_with_conflict() {
    let root = temp_root("illegal");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let id = run_id("illegal");
    store
        .create_run(NewRun {
            id: id.clone(),
            capabilities: flags(),
            budgets: budgets(),
        })
        .await
        .unwrap();
    // No implicit approval: planned → executing skips the gate.
    let error = store
        .set_run_status(&id, RunStatus::Executing, None)
        .await
        .unwrap_err();
    assert_eq!(error, StoreError::Conflict);
    store
        .set_run_status(&id, RunStatus::Cancelled, None)
        .await
        .unwrap();
    // A terminal status never leaves.
    for status in [RunStatus::Executing, RunStatus::Paused, RunStatus::Failed] {
        let error = store.set_run_status(&id, status, None).await.unwrap_err();
        assert_eq!(
            error,
            StoreError::Conflict,
            "{status:?} left a cancelled run"
        );
    }
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn terminal_failure_carries_a_typed_code() {
    let root = temp_root("failure");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let id = run_id("failure");
    store
        .create_run(NewRun {
            id: id.clone(),
            capabilities: flags(),
            budgets: budgets(),
        })
        .await
        .unwrap();
    // Walk to executing: only a run that is executing can fail.
    store
        .set_run_status(&id, RunStatus::Approved, None)
        .await
        .unwrap();
    store
        .set_run_status(&id, RunStatus::Executing, None)
        .await
        .unwrap();
    // A failure without its typed cause is not a terminal state.
    let error = store
        .set_run_status(&id, RunStatus::Failed, None)
        .await
        .unwrap_err();
    assert_eq!(error, StoreError::Invalid);
    // A code on a non-failure state is meaningless, even on a legal transition.
    let error = store
        .set_run_status(&id, RunStatus::Paused, Some(RunFailureCode::Provider))
        .await
        .unwrap_err();
    assert_eq!(error, StoreError::Invalid);
    store
        .set_run_status(&id, RunStatus::Paused, None)
        .await
        .unwrap();
    store
        .set_run_status(&id, RunStatus::Executing, None)
        .await
        .unwrap();
    store
        .set_run_status(&id, RunStatus::Failed, Some(RunFailureCode::SafetyQuery))
        .await
        .unwrap();
    let record = store.get_run(&id).await.unwrap().unwrap();
    assert_eq!(record.status, RunStatus::Failed);
    assert_eq!(record.failure_code, Some(RunFailureCode::SafetyQuery));
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Steps
// ---------------------------------------------------------------------------

#[tokio::test]
async fn steps_walk_pending_running_done_and_bounded_retry() {
    let root = temp_root("steps");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let id = run_id("steps");
    store
        .create_run(NewRun {
            id: id.clone(),
            capabilities: flags(),
            budgets: budgets(),
        })
        .await
        .unwrap();
    // A step is born pending or running, never already done.
    let error = store
        .upsert_step(&id, 0, RunStepStatus::Done)
        .await
        .unwrap_err();
    assert_eq!(error, StoreError::Invalid);
    store
        .upsert_step(&id, 0, RunStepStatus::Pending)
        .await
        .unwrap();
    store
        .upsert_step(&id, 0, RunStepStatus::Running)
        .await
        .unwrap();
    store
        .upsert_step(&id, 0, RunStepStatus::Done)
        .await
        .unwrap();
    // A failed step may restart (the engine's bounded retry).
    store
        .upsert_step(&id, 1, RunStepStatus::Running)
        .await
        .unwrap();
    store
        .upsert_step(&id, 1, RunStepStatus::Failed)
        .await
        .unwrap();
    store
        .upsert_step(&id, 1, RunStepStatus::Running)
        .await
        .unwrap();
    let steps = store.list_steps(&id).await.unwrap();
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].status, RunStepStatus::Done);
    assert_eq!(steps[1].status, RunStepStatus::Running);
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn steps_of_an_unknown_run_are_not_found() {
    let root = temp_root("orphan-step");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let error = store
        .upsert_step(&run_id("ghost"), 0, RunStepStatus::Pending)
        .await
        .unwrap_err();
    assert_eq!(error, StoreError::NotFound);
    let steps = store.list_steps(&run_id("ghost")).await.unwrap();
    assert!(steps.is_empty());
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Usage: nullable, unknown ≠ zero
// ---------------------------------------------------------------------------

#[tokio::test]
async fn usage_is_null_when_unknown_and_never_zero() {
    let root = temp_root("usage");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let id = run_id("usage");
    store
        .create_run(NewRun {
            id: id.clone(),
            capabilities: flags(),
            budgets: budgets(),
        })
        .await
        .unwrap();
    store
        .upsert_step(&id, 0, RunStepStatus::Running)
        .await
        .unwrap();
    // Unknown usage at the API, and literally NULL in the row: absence must
    // never be read back as zero.
    let record = store.get_run(&id).await.unwrap().unwrap();
    assert_eq!(record.usage, RunUsage::unknown());
    {
        let pool = read_pool(&db).await;
        let turns: Option<i64> = sqlx::query_scalar("SELECT usage_turns FROM runs WHERE id = ?")
            .bind(id.as_str())
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(turns, None, "unknown usage was stored as a number");
        let step_turns: Option<i64> =
            sqlx::query_scalar("SELECT usage_turns FROM run_steps WHERE run_id=? AND step=0")
                .bind(id.as_str())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            step_turns, None,
            "unknown step usage was stored as a number"
        );
        pool.close().await;
    }
    // Reporting unknown again — an explicitly unreported figure for the
    // primary endpoint — keeps the figures unknown, not zero.
    store.set_run_usage(&id, usage_unknown()).await.unwrap();
    let record = store.get_run(&id).await.unwrap().unwrap();
    assert_eq!(record.usage, usage_unknown());
    // A known figure round-trips through the API and the bytes.
    store.set_run_usage(&id, usage_known()).await.unwrap();
    let record = store.get_run(&id).await.unwrap().unwrap();
    assert_eq!(record.usage, usage_known());
    store.set_step_usage(&id, 0, usage_known()).await.unwrap();
    let steps = store.list_steps(&id).await.unwrap();
    assert_eq!(steps[0].usage, usage_known());
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn usage_for_missing_records_is_not_found() {
    let root = temp_root("usage-missing");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let error = store
        .set_run_usage(&run_id("ghost"), usage_known())
        .await
        .unwrap_err();
    assert_eq!(error, StoreError::NotFound);
    let error = store
        .set_run_status(&run_id("ghost"), RunStatus::Approved, None)
        .await
        .unwrap_err();
    assert_eq!(error, StoreError::NotFound);
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// The persisted-string gate on the endpoint-key channels
// ---------------------------------------------------------------------------

#[tokio::test]
async fn credential_shaped_endpoint_keys_are_refused_and_never_stored() {
    let root = temp_root("admission");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    // The clean run first, so the database and its sidecars exist and the
    // byte scan below has real bytes to scan.
    store
        .create_run(NewRun {
            id: run_id("clean"),
            capabilities: flags(),
            budgets: budgets(),
        })
        .await
        .unwrap();
    let mut secret_budgets = budgets();
    secret_budgets
        .tokens_per_endpoint
        .insert("api_key=sk-live-SENTINELKEY".to_owned(), 1_000);
    let error = store
        .create_run(NewRun {
            id: run_id("admission"),
            capabilities: flags(),
            budgets: secret_budgets,
        })
        .await
        .unwrap_err();
    assert_eq!(error, StoreError::Invalid);
    // The refusal happened before any INSERT: the sentinel is in no byte.
    assert_absent("run store", &db_bytes(&db), "SENTINELKEY");
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn unshaped_endpoint_keys_and_overwide_maps_are_refused() {
    let root = temp_root("keys");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let mut control_budgets = budgets();
    control_budgets
        .tokens_per_endpoint
        .insert("bad\u{7}key".to_owned(), 1);
    let error = store
        .create_run(NewRun {
            id: run_id("control"),
            capabilities: flags(),
            budgets: control_budgets,
        })
        .await
        .unwrap_err();
    assert_eq!(error, StoreError::Invalid);
    let mut wide_budgets = RunBudgets::default();
    for index in 0..9 {
        wide_budgets
            .tokens_per_endpoint
            .insert(format!("endpoint-{index}"), 1);
    }
    let error = store
        .create_run(NewRun {
            id: run_id("wide"),
            capabilities: flags(),
            budgets: wide_budgets,
        })
        .await
        .unwrap_err();
    // A map over the endpoint bound exceeds a store limit, not a shape fault.
    assert_eq!(error, StoreError::LimitExceeded);
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Structural privacy: payload-free bytes, metadata-only schema
// ---------------------------------------------------------------------------

#[tokio::test]
async fn goal_sentinel_never_reaches_the_bytes() {
    let root = temp_root("sentinel");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    // Drive the full flow the engine will drive. The goal below carries the
    // sentinels; the API exposes no parameter it could travel through.
    let goal = GOAL_SENTINELS.join(" and ");
    assert!(!goal.is_empty());
    let id = run_id("sentinel");
    store
        .create_run(NewRun {
            id: id.clone(),
            capabilities: flags(),
            budgets: budgets(),
        })
        .await
        .unwrap();
    store
        .set_run_status(&id, RunStatus::Approved, None)
        .await
        .unwrap();
    store
        .set_run_status(&id, RunStatus::Executing, None)
        .await
        .unwrap();
    store
        .upsert_step(&id, 0, RunStepStatus::Running)
        .await
        .unwrap();
    store.set_step_usage(&id, 0, usage_known()).await.unwrap();
    store
        .upsert_step(&id, 0, RunStepStatus::Done)
        .await
        .unwrap();
    store.set_run_usage(&id, usage_known()).await.unwrap();
    store
        .set_run_status(&id, RunStatus::Completed, None)
        .await
        .unwrap();
    // The flow really wrote rows — the scan below is not vacuous.
    let record = store.get_run(&id).await.unwrap().unwrap();
    assert_eq!(record.status, RunStatus::Completed);
    let bytes = db_bytes(&db);
    for sentinel in GOAL_SENTINELS {
        assert_absent("run store", &bytes, sentinel);
    }
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

/// Every column the run tables may carry. A new column must be added here
/// consciously — the structural guard that keeps goal/plan payload columns
/// out of the metadata store, and that keeps this test permanently red if one
/// ever appears.
const RUNS_COLUMNS: &[&str] = &[
    "id",
    "status",
    "failure_code",
    "cap_workspace_write",
    "cap_fetch",
    "cap_runner",
    "cap_scratch",
    "budget_wall_clock_ms",
    "budget_tokens_json",
    "budget_turns",
    "budget_tool_calls",
    "budget_downloaded_bytes",
    "budget_workspace_bytes",
    "budget_workspace_files",
    "budget_process_count",
    "budget_process_time_ms",
    "usage_wall_clock_ms",
    "usage_tokens_json",
    "usage_turns",
    "usage_tool_calls",
    "created_unix_ms",
    "updated_unix_ms",
];
const RUN_STEPS_COLUMNS: &[&str] = &[
    "run_id",
    "step",
    "status",
    "usage_wall_clock_ms",
    "usage_tokens_json",
    "usage_turns",
    "usage_tool_calls",
    "created_unix_ms",
    "updated_unix_ms",
];

#[tokio::test]
async fn run_schema_stays_metadata_only() {
    let root = temp_root("schema");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store.list_schema_metadata().await.unwrap();
    store.close().await;
    for (table, allowlist) in [("runs", RUNS_COLUMNS), ("run_steps", RUN_STEPS_COLUMNS)] {
        let pool = read_pool(&db).await;
        let mut columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info(?) ORDER BY name")
                .bind(table)
                .fetch_all(&pool)
                .await
                .unwrap();
        pool.close().await;
        let mut expected: Vec<&str> = allowlist.to_vec();
        expected.sort_unstable();
        columns.sort();
        assert_eq!(
            columns, expected,
            "{table} gained a column outside the metadata allowlist"
        );
    }
    let _ = fs::remove_dir_all(root);
}
