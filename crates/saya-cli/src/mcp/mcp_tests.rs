//! Unit tests for the MCP serve policy (task Da + Db): the allowlist rule
//! (explicit `--profile` values, else the configured default, else none), the
//! byte bounds, the in-flight cap, the profiles payload shape, and the
//! catalog's data-sharing gate. The wire behavior is pinned in
//! `tests/mcp_stdio.rs` against the real binary.

use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use super::policy::{MAX_IN_FLIGHT, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES, ServePolicy};
use super::replay_cancellation::{
    ReplayCompletion, ReplayStop, acquire_replay_slot, retain_replay,
};
use super::server::SayaServer;
use crate::commands::{capture_output_start, capture_output_take};
use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;
use saya_config::{CliOverrides, ConfigFile, ConnectionsFile, ResolutionInput, resolve};
use saya_store::{InvestigationRepository, LocalBinding};
use saya_types::InvestigationId;

fn fixture_runtime(connections_toml: &str, config_toml: &str) -> RuntimeConfig {
    let connections = ConnectionsFile::from_toml(connections_toml).unwrap();
    let user = ConfigFile::from_toml(config_toml).unwrap();
    let resolved = resolve(
        ResolutionInput::new(connections.clone())
            .with_user(user)
            .with_cli(CliOverrides::default()),
    )
    .unwrap();
    RuntimeConfig {
        resolved,
        connections,
        config_path: None,
        connections_path: None,
        cache_scope: PathBuf::new(),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: BTreeMap::new(),
    }
}

const TWO_PROFILES: &str = "[profiles.first]\ntype = 'sqlite'\npath = 'a.sqlite3'\n\n\
                            [profiles.second]\ntype = 'sqlite'\npath = 'b.sqlite3'\n";

include!("mcp_tests/policy.rs");

fn second(name: &str) -> super::policy::ProfileSummary {
    super::policy::ProfileSummary {
        name: name.to_owned(),
        dialect: "sqlite",
    }
}

include!("mcp_tests/replay_resolution.rs");

include!("mcp_tests/queued_replay.rs");

/// Seeds an `events` table with one labelled row into a sqlite file, the
/// way the binary-level fixtures do; only the test writes.
async fn seed_events(database: &std::path::Path, label: &str) {
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(database)
        .create_if_missing(true);
    let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
    sqlx::query("CREATE TABLE events (id INTEGER PRIMARY KEY, label TEXT NOT NULL)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO events (id, label) VALUES (1, ?)")
        .bind(label)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

/// Saves one investigation through the dispatcher (save never connects) and
/// returns its id, with the save's own rendering captured. The dispatcher
/// derives its repository from `runtime.investigations_root`, the same root
/// the test's own `repo` reads.
async fn saved_id(
    runtime: &crate::config::runtime::RuntimeConfig,
    state_db: &saya_store::SqliteStateStore,
) -> String {
    capture_output_start();
    let outcome = crate::commands::run_investigation_outcome(
        crate::cli::InvestigationCommand::Save {
            name: "remap probe".into(),
            description: None,
            sql: Some("SELECT label FROM events".into()),
            file: None,
            connection: Some("local".into()),
            param_specs: Vec::new(),
        },
        runtime,
        RenderFormat::Text,
        false,
        state_db,
    )
    .await
    .unwrap();
    let (out, err) = capture_output_take();
    assert_eq!(outcome.code, 0, "save failed: {out}{err}");
    out.lines()
        .next()
        .expect("save prints the id first")
        .trim()
        .to_string()
}

include!("mcp_tests/replay_cancellation.rs");

include!("mcp_tests/unbound_replay.rs");

include!("mcp_tests/stale_contract.rs");
