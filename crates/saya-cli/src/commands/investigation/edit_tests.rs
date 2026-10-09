//! Unit tests for `saya investigation edit` (A2.5 reopened): an edit is a
//! new revision through the shared dispatcher (`run_investigation_in`), the
//! same save-time gates guard the replacement SQL, the id, dialect, and
//! connection alias never change, and the local review binding is left
//! stale by revision — the next run refuses until `--revalidate`. The
//! harness mirrors `tests.rs` but is self-contained: those helpers are
//! private to that module.

use crate::cli::InvestigationCommand;
use crate::commands::investigation::run_investigation_in;
use crate::commands::{capture_output_start, capture_output_take};
use crate::config::runtime::{RuntimeConfig, load_with_sources};
use crate::render::RenderFormat;
use saya_store::{InvestigationRepository, SqliteStateStore};
use saya_types::SqlDialect;
use saya_types::investigation::InvestigationDefinitionV1;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

// -- harness ---------------------------------------------------------------

fn temp_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "saya-investigation-edit-unit-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// A single-profile runtime over a sqlite database file; the one-profile
/// connections file auto-selects `local` as the active profile, so save
/// resolves it without `--connection`.
fn runtime_at(root: &Path, connections_toml: &str) -> RuntimeConfig {
    let connections = root.join("connections.toml");
    fs::write(&connections, connections_toml).unwrap();
    let options = crate::cli::GlobalOptions {
        connections: Some(connections),
        ..Default::default()
    };
    load_with_sources(&options, root, root, BTreeMap::new()).unwrap()
}

fn single_profile_toml(root: &Path) -> String {
    let database = root.join("data.sqlite3");
    fs::write(&database, b"").unwrap();
    format!(
        "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n",
        database.display()
    )
}

async fn run(
    repo: &InvestigationRepository,
    command: InvestigationCommand,
    runtime: &RuntimeConfig,
    state_db: &SqliteStateStore,
) -> (i32, String, String) {
    capture_output_start();
    let code = run_investigation_in(repo, command, runtime, RenderFormat::Text, false, state_db)
        .await
        .unwrap();
    let (out, err) = capture_output_take();
    (code, out, err)
}

fn repo_at(root: &Path) -> InvestigationRepository {
    InvestigationRepository::new(root.join("investigations"))
}

fn document_path(root: &Path, id: &str) -> PathBuf {
    root.join("investigations").join(format!("{id}.json"))
}

fn binding_path(root: &Path, id: &str) -> PathBuf {
    root.join("investigations")
        .join("local")
        .join(format!("{id}.json"))
}

async fn saved_id(
    repo: &InvestigationRepository,
    runtime: &RuntimeConfig,
    state: &SqliteStateStore,
) -> String {
    let (code, out, err) = run(
        repo,
        InvestigationCommand::Save {
            name: "Order events".into(),
            description: Some("events in order".into()),
            sql: Some("SELECT id, label FROM events ORDER BY id".into()),
            file: None,
            connection: None,
            param_specs: Vec::new(),
        },
        runtime,
        state,
    )
    .await;
    assert_eq!(code, 0, "save failed: {out}{err}");
    out.lines()
        .next()
        .expect("save prints the id first")
        .to_string()
}

fn edit_command(
    id: &str,
    name: Option<&str>,
    description: Option<&str>,
    sql: Option<&str>,
    file: Option<PathBuf>,
) -> InvestigationCommand {
    InvestigationCommand::Edit {
        id: id.to_string(),
        name: name.map(Into::into),
        description: description.map(Into::into),
        sql: sql.map(Into::into),
        file,
        param_specs: Vec::new(),
    }
}

fn run_command(id: &str, revalidate: bool) -> InvestigationCommand {
    InvestigationCommand::Run {
        id: id.to_string(),
        connection: None,
        revalidate,
        report: None,
        rows: None,
        overwrite: false,
        params: Vec::new(),
    }
}

/// Seeds the `events` table straight into the profile's sqlite file, so a
/// replay after `--revalidate` has something bounded to read.
async fn seed_events(database: &Path) {
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(database)
        .create_if_missing(true);
    let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
    sqlx::query("CREATE TABLE events (id INTEGER PRIMARY KEY, label TEXT NOT NULL)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO events (id, label) VALUES (1, 'first'), (2, 'second')")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

// -- edit ------------------------------------------------------------------

include!("edit_tests/revision_and_replacement.rs");

include!("edit_tests/safety_and_usage.rs");

include!("edit_tests/conflict_and_name.rs");
