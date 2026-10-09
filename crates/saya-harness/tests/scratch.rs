//! The `scratch_sql` battery (M4-2): the red tests for the amended ADR 0003
//! decision. The validator is the thing under test — every file-reading
//! statement is refused by it, by name, before the statement reaches DuckDB;
//! DuckDB's own permission-layer refusal is asserted separately, as the
//! backstop. The battery runs both directions: every statement `scratch_sql`
//! accepts stays inside the run dir (the sentinel planted outside is never
//! touched), and every write it accepts is still rejected by the user-DB
//! `prepare_*` functions, unmodified.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use saya_agent::{
    CancellationToken, LocalStateEffect, MAX_TOOL_MESSAGE_BYTES, ToolError, ToolExecutionContext,
    ToolExecutor,
};
use saya_connectors::{
    prepare_bigquery_sql, prepare_clickhouse_sql, prepare_duckdb_sql, prepare_mysql_sql,
    prepare_postgres_sql, prepare_snowflake_sql, prepare_sqlite_sql,
};
use saya_harness::{
    run_dir::RunDir,
    scratch::{
        MAX_SQL_BYTES, SCRATCH_FILE_NAME, ScratchError, ScratchRejection, ScratchSql, validate,
    },
};
use saya_types::{Capabilities, RunId};
use serde_json::{Value, json};

const SENTINEL_BYTES: &[u8] = b"sentinel-MUST-NOT-LEAK";

/// A run directory under a temp runs root, created by the production
/// [`RunDir::create`] (0700), plus a sentinel planted *outside* the run dir —
/// a real readable file any file-reading carve-out would find.
struct TempRun {
    runs_root: PathBuf,
    run_dir: PathBuf,
}

impl TempRun {
    fn new(tag: &str) -> Self {
        let runs_root =
            std::env::temp_dir().join(format!("saya-scratch-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&runs_root);
        fs::create_dir_all(&runs_root).unwrap();
        let id = RunId::parse("scratch-battery").unwrap();
        let dir = RunDir::create(&runs_root, &id).unwrap();
        Self {
            runs_root,
            run_dir: dir.root().to_path_buf(),
        }
    }

    fn root(&self) -> &Path {
        &self.run_dir
    }

    /// The sentinel: in the runs root, a sibling of the run dir — outside it.
    fn plant_sentinel(&self) -> PathBuf {
        let path = self.runs_root.join("sentinel.csv");
        fs::write(&path, SENTINEL_BYTES).unwrap();
        path
    }

    fn sentinel_untouched(&self, path: &Path) {
        assert_eq!(
            fs::read(path).unwrap(),
            SENTINEL_BYTES,
            "the sentinel outside the run dir must never be touched"
        );
    }

    fn scratch_file(&self) -> PathBuf {
        self.run_dir.join(SCRATCH_FILE_NAME)
    }
}

impl Drop for TempRun {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.runs_root);
    }
}

fn scratch_approved() -> Capabilities {
    let mut caps = Capabilities::default();
    caps.scratch = true;
    caps
}

#[path = "scratch/limits.rs"]
mod limits;

/// The rows of a [`QueryResult`]-shaped expectation, as a `Vec<Value>` for
/// direct comparison.
fn json_rows(rows: Value) -> Vec<Value> {
    rows.as_array().expect("row array").clone()
}

fn admitted(run: &TempRun) -> ScratchSql {
    ScratchSql::admit(run.root(), &scratch_approved())
        .unwrap()
        .expect("the scratch scope is approved")
}

/// Drives the async tool from the property battery's synchronous closure.
fn run_sync(tool: &ScratchSql, sql: &str) -> Result<saya_types::QueryResult, ScratchError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(tool.run(sql))
}

// ---------------------------------------------------------------------------
// 2. The file-reading family is refused by the validator, by name, before it
//    reaches DuckDB — and DuckDB refuses it too, asserted separately.
// ---------------------------------------------------------------------------

#[path = "scratch/validator_refusals.rs"]
mod validator_refusals;

// ---------------------------------------------------------------------------
// 3. INSTALL/LOAD refused by the validator.

// ---------------------------------------------------------------------------
// 4. Multi-statement input refused.

// ---------------------------------------------------------------------------
// 5. The capability survives: CREATE TABLE / INSERT / SELECT round-trip.

#[path = "scratch/bounded_queries.rs"]
mod bounded_queries;

// ---------------------------------------------------------------------------
// 6. Structural: scratch never travels the DatabaseConnector path.

/// The source with line comments (`//`, `//!`, `///`) stripped, so the scan
/// sees only code.
fn code_only(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn manifest_section(manifest: &str, header: &str) -> String {
    let start = manifest.find(header).map(|i| i + header.len()).unwrap_or(0);
    let end = manifest[start..]
        .find("\n[")
        .map(|i| start + i)
        .unwrap_or(manifest.len());
    manifest[start..end].to_string()
}

// ---------------------------------------------------------------------------
// 7. The created file is 0600 after open.

#[path = "scratch/scope_and_arguments.rs"]
mod scope_and_arguments;

// ---------------------------------------------------------------------------
// Admission: hidden-not-advertised until the scratch scope is approved.

// ---------------------------------------------------------------------------
// Timeout plus interrupt-then-await: exactly the connector's shape.

#[path = "scratch/cancellation.rs"]
mod cancellation;

// ---------------------------------------------------------------------------
// Turn cancellation: a cancelled turn stops a running scratch statement —
// the fetch tool's shape (`execute_with_context` over the turn's token)
// carried onto the scratch engine. Every await sits under a 15 s guard so a
// regression hangs into a failure instead of stalling the suite.

/// The loop-assigned context: the loop's tool-message cap and the turn's
/// token. The scratch tool applies its own row and byte budgets internally,
/// so the cap rides along unconsulted — exactly as in the loop.
fn context(token: &CancellationToken) -> ToolExecutionContext {
    ToolExecutionContext {
        result_cap: MAX_TOOL_MESSAGE_BYTES,
        cancellation: token.clone(),
    }
}

// ---------------------------------------------------------------------------
// 1. The property battery, both directions.

#[path = "scratch/property.rs"]
mod property;

/// Reopens the scratch file DuckDB-side (after the tool closed it) to inspect
/// what the engine actually holds.
fn reopened_scratch(run: &TempRun) -> duckdb::Connection {
    let config = duckdb::Config::default()
        .access_mode(duckdb::AccessMode::ReadWrite)
        .and_then(|item| item.enable_external_access(false))
        .and_then(|item| item.enable_autoload_extension(false))
        .and_then(|item| item.with("allow_community_extensions", "false"))
        .and_then(|item| item.with("allow_persistent_secrets", "false"))
        .and_then(|item| item.with("lock_configuration", "true"))
        .expect("the pinned scratch configuration must build");
    duckdb::Connection::open_with_flags(run.scratch_file(), config).unwrap()
}
