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

// ---------------------------------------------------------------------------
// Status transitions
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Steps
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Usage: nullable, unknown ≠ zero
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// The persisted-string gate on the endpoint-key channels
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Structural privacy: payload-free bytes, metadata-only schema
// ---------------------------------------------------------------------------

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

#[path = "runs/creation.rs"]
mod creation;
#[path = "runs/lifecycle.rs"]
mod lifecycle;
#[path = "runs/privacy.rs"]
mod privacy;
#[path = "runs/steps.rs"]
mod steps;
#[path = "runs/usage.rs"]
mod usage;
