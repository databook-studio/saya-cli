//! Tests for the configuration reload core: a candidate built without side
//! effects, a plan that classifies every change, and re-seeds only what the
//! user never overrode. Authority-sensitive settings (data sharing, host
//! command shaping) must never move for a running session.

use super::classify::{CLASS_TABLE, Class, DEFAULTS_TO_PENDING, classify_change, classify_key};
use super::plan::{ChangeLine, ReloadPlan, StateReseed};
use super::{ReloadInputs, apply_plan, build_candidate, plan_reload};
use crate::cli::GlobalOptions;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::SessionState;
use saya_config::{ConfigFile, ConnectionsFile, ResolutionInput, resolve};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

fn resolved_from(
    user_toml: &str,
    project_toml: Option<&str>,
    connections_toml: &str,
) -> saya_config::ResolvedConfig {
    let connections = ConnectionsFile::from_toml(connections_toml).unwrap();
    let mut input =
        ResolutionInput::new(connections).with_user(ConfigFile::from_toml(user_toml).unwrap());
    if let Some(project) = project_toml {
        input = input.with_project(ConfigFile::from_toml(project).unwrap());
    }
    resolve(input).unwrap()
}

fn runtime_from(config_toml: &str, connections_toml: &str) -> RuntimeConfig {
    runtime_from_layers(config_toml, None, connections_toml)
}

fn runtime_from_layers(
    user_toml: &str,
    project_toml: Option<&str>,
    connections_toml: &str,
) -> RuntimeConfig {
    RuntimeConfig {
        resolved: resolved_from(user_toml, project_toml, connections_toml),
        connections: ConnectionsFile::from_toml(connections_toml).unwrap(),
        config_path: None,
        connections_path: None,
        cache_scope: PathBuf::from("/tmp/saya-reload-test"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: BTreeMap::new(),
    }
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "saya-reload-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn state_with(model: &str) -> SessionState {
    SessionState::new("s1", None, model)
}

// --- build_candidate ---

include!("reload_tests/source_precedence.rs");

include!("reload_tests/live_and_reseed.rs");

include!("reload_tests/session_and_profile_changes.rs");

include!("reload_tests/endpoint_changes.rs");

include!("reload_tests/rendering_and_launch.rs");
