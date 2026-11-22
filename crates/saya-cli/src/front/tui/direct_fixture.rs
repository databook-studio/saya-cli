//! Test support for the direct commands: a fixture SQLite profile, a composed
//! runtime over it, and a trust store that records instead of binding.
//!
//! Everything a direct command touches is real here — the query runs through
//! the same safety layer a run's does — so the tests assert what the surface
//! gets back, not what a double returned. Only the trust store is a double:
//! acquiring a live session to bind a folder is not what these tests are about.
//!
//! The profile is one temp tree under `./target/tmp-direct/`, named for the
//! test and the process, so two runs never share a database.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ratatui::crossterm::event::KeyCode;
use saya_store::SqliteStateStore;
use saya_types::{EvidenceSource, ExecutionEvidence, QueryResult, ResultScope, SqlDialect};

use super::direct_run::DirectDeps;
use super::direct_trust::TrustStore;
use super::live::LiveRuntime;
use super::model::Model;
use super::msg::{Cmd, Msg};
use super::run_fixture::key;
use super::update::update;
use crate::config::runtime::{RuntimeConfig, load_with_sources};
use crate::front::ResultPreview;
use crate::render::RenderFormat;

/// The profile every fixture test names.
pub(crate) const PROFILE: &str = "local";

/// One fixture: its own tree, and what a direct command needs from a launch.
pub(crate) struct Fixture {
    pub(crate) root: PathBuf,
    pub(crate) deps: Arc<DirectDeps>,
    pub(crate) trust: Arc<RecordingTrust>,
}

impl Fixture {
    /// A fresh tree with the two-event table, and a runtime over it.
    pub(crate) fn new(label: &str) -> Self {
        // The workspace's own `target/`, so a fixture never writes outside the
        // tree the build already owns and its files never reach a commit.
        let root = workspace_root()
            .join("target/tmp-direct")
            .join(format!("{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("the fixture's tree is made");
        let database = root.join("data.sqlite3");
        seed(&database);
        let connections = root.join("connections.toml");
        std::fs::write(
            &connections,
            format!(
                "[profiles.{PROFILE}]\ntype = 'sqlite'\npath = '{}'\n",
                database.display()
            ),
        )
        .expect("the fixture's connections file is written");
        let options = crate::cli::GlobalOptions {
            connections: Some(connections),
            ..Default::default()
        };
        let user = root.join("user-config");
        std::fs::create_dir_all(&user).expect("the fixture's user config dir is made");
        let runtime: RuntimeConfig = load_with_sources(&options, &root, &user, BTreeMap::new())
            .expect("the fixture runtime loads");
        let trust = Arc::new(RecordingTrust::default());
        let deps = Arc::new(DirectDeps {
            runtime: Arc::new(LiveRuntime::new(Arc::new(runtime))),
            state_db: SqliteStateStore::new(root.join("state.sqlite3")),
            format: RenderFormat::Text,
            trust: trust.clone(),
        });
        Self { root, deps, trust }
    }

    /// The same dependencies, with the investigations root inside the
    /// fixture's own tree. A composed runtime resolves that root from the
    /// environment (`SAYA_INVESTIGATIONS_DIR`), else beside the state DB — so a
    /// test that saves a document must point it here, or it writes the
    /// operator's real root.
    pub(crate) fn investigation_deps(&self) -> Arc<DirectDeps> {
        let mut runtime = (*self.deps.runtime.current()).clone();
        runtime.investigations_root = self.root.join("investigations");
        Arc::new(DirectDeps {
            runtime: Arc::new(super::live::LiveRuntime::new(Arc::new(runtime))),
            state_db: self.deps.state_db.clone(),
            format: self.deps.format,
            trust: self.deps.trust.clone(),
        })
    }

    /// The surface's model, on the fixture's connection.
    pub(crate) fn model(&self) -> Model {
        Model {
            database: Some(PROFILE.to_owned()),
            ..Model::default()
        }
    }
}

/// The SQL the report fixtures state, so a written document is pinned by
/// content rather than by shape.
pub(crate) const REPORT_SQL: &str = "SELECT id, label FROM events ORDER BY id";

/// The two events the fixture's own database holds, as a result.
pub(crate) fn report_result() -> QueryResult {
    QueryResult {
        columns: vec!["id".to_owned(), "label".to_owned()],
        rows: vec![serde_json::json!([1, "one"]), serde_json::json!([2, "two"])],
        row_count: 2,
        truncated: false,
        executed_sql: REPORT_SQL.to_owned(),
    }
}

/// The provenance of [`report_result`]: the facts a report's Provenance
/// section states, spelled out so two documents compare byte for byte.
pub(crate) fn report_evidence() -> ExecutionEvidence {
    ExecutionEvidence {
        execution_id: "9f8e7d6c5b4a".to_owned(),
        submitted_sql_sha256: "0a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f9"
            .to_owned(),
        connection_label: "local".to_owned(),
        connection_identity: Some("sqlite:data.sqlite3".to_owned()),
        dialect: SqlDialect::Sqlite,
        schema_fingerprint: None,
        started_unix_ms: 1_700_000_000_000,
        finished_unix_ms: 1_700_000_000_250,
        max_rows: 1000,
        returned_rows: 2,
        truncated: false,
        scope: ResultScope::Full,
        source: EvidenceSource::DirectSql,
        knowledge_ids: Vec::new(),
        param_names: Vec::new(),
        params_sha256: None,
    }
}

/// The result entry the surface holds, as a landed `/sql` leaves it.
pub(crate) fn held_preview() -> ResultPreview {
    ResultPreview {
        connection: "local".to_owned(),
        sql: REPORT_SQL.to_owned(),
        result: report_result(),
        evidence: Some(std::sync::Arc::new(report_evidence())),
        refusal: None,
    }
}

/// Submits one line to the surface and answers the commands it decided on.
pub(crate) fn submit(model: &mut Model, line: &str) -> Vec<Cmd> {
    let mut cmds = update(model, Msg::Paste(line.to_owned()));
    cmds.extend(update(model, key(KeyCode::Enter)));
    cmds
}

/// A trust store that records what it was asked to bind, and binds nothing.
#[derive(Debug, Default)]
pub(crate) struct RecordingTrust {
    bound: Mutex<Vec<PathBuf>>,
}

impl RecordingTrust {
    /// The folders this store was asked to bind, in order.
    pub(crate) fn bound(&self) -> MutexGuard<'_, Vec<PathBuf>> {
        self.bound.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl TrustStore for RecordingTrust {
    fn bind(&self, dir: &Path) -> Result<(), String> {
        self.bound
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(dir.to_path_buf());
        Ok(())
    }
}

/// The workspace root this crate is built in: `crates/saya-cli`'s two parents.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the crate sits two levels under the workspace root")
        .to_path_buf()
}

/// Writes the two-event table the query tests read.
fn seed(database: &Path) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a test runtime builds");
    runtime.block_on(async {
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(database)
            .create_if_missing(true);
        let pool = sqlx::SqlitePool::connect_with(options)
            .await
            .expect("the fixture database opens");
        sqlx::query("CREATE TABLE events (id INTEGER, label TEXT)")
            .execute(&pool)
            .await
            .expect("the fixture table is made");
        sqlx::query("INSERT INTO events (id, label) VALUES (1, 'one'), (2, 'two')")
            .execute(&pool)
            .await
            .expect("the fixture rows are written");
        pool.close().await;
    });
}
