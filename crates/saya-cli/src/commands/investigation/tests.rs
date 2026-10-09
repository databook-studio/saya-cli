//! Unit tests for the investigation command operations (S6). The dispatcher
//! is driven with an explicit repository (`run_investigation_in`) so no test
//! touches process env; the binary-level surface is covered in
//! `tests/investigation_cli.rs`.

use super::{RunOutcome, run_investigation_in};
use crate::cli::InvestigationCommand;
use crate::commands::{capture_output_start, capture_output_take};
use crate::config::runtime::{RuntimeConfig, load_with_sources};
use crate::profile_identity::profile_identity;
use crate::render::RenderFormat;
use saya_agent::CancellationToken;
use saya_store::{AuditStore, InvestigationRepository, SqliteStateStore};
use saya_types::investigation::InvestigationDefinitionV1;
use saya_types::{EvidenceSource, InvestigationId, ResultScope, SqlDialect};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

// -- harness ---------------------------------------------------------------

fn temp_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "saya-investigation-cli-unit-{label}-{}",
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
) -> (i32, String, String) {
    capture_output_start();
    // The dispatcher threads a state store for the async `run` arm only;
    // these sync-arm tests never touch it, so a scratch path suffices.
    let state_db =
        SqliteStateStore::new(std::env::temp_dir().join("saya-investigation-unit-state"));
    let code = run_investigation_in(repo, command, runtime, RenderFormat::Text, false, &state_db)
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

/// Saves one investigation and returns its id, for tests that start from a
/// saved state.
async fn saved_id(repo: &InvestigationRepository, runtime: &RuntimeConfig) -> String {
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
    )
    .await;
    assert_eq!(code, 0, "save failed: {out}{err}");
    out.lines()
        .next()
        .expect("save prints the id first")
        .to_string()
}

fn expected_identity(runtime: &RuntimeConfig, name: &str) -> String {
    let profile = runtime.named_profile(name).unwrap();
    profile_identity(name, profile, &runtime.cache_scope)
        .as_str()
        .to_owned()
}

// -- save ------------------------------------------------------------------

include!("tests/save.rs");

// -- list ------------------------------------------------------------------

include!("tests/list.rs");

// -- show ------------------------------------------------------------------

include!("tests/show.rs");

// -- delete ----------------------------------------------------------------

include!("tests/delete.rs");

// -- run outcome (C3/D12) --------------------------------------------------

include!("tests/run_support.rs");
include!("tests/run_outcomes.rs");

include!("tests/run_cancellation_and_unbound.rs");

include!("tests/run_bindings.rs");
