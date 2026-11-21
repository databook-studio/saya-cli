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

use super::direct_run::DirectDeps;
use super::direct_trust::TrustStore;
use super::model::Model;
use super::msg::{Cmd, Msg};
use super::run_fixture::key;
use super::update::update;
use crate::config::runtime::{RuntimeConfig, load_with_sources};
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
            runtime: Arc::new(runtime),
            state_db: SqliteStateStore::new(root.join("state.sqlite3")),
            format: RenderFormat::Text,
            trust: trust.clone(),
        });
        Self { root, deps, trust }
    }

    /// The surface's model, on the fixture's connection.
    pub(crate) fn model(&self) -> Model {
        Model {
            database: Some(PROFILE.to_owned()),
            ..Model::default()
        }
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
