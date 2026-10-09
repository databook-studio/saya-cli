//! Unit tests for `config show --explain`: the winning layer of every
//! user-facing setting, read back through the same merge funnel that resolved
//! the value. The invariants under test: every key is a user-facing key in
//! table order, the layer names the variable/flag for env/CLI sources, file
//! layers name the path a user would edit, the ignored project overrides
//! carry the pinned reason, the three `[jobs.runner]` rows display distinct
//! labels over their one grouped origin, and no secret value ever appears —
//! in JSON or in the text rendering.

use std::collections::BTreeMap;
use std::path::PathBuf;

use super::{ConfigExplanation, IgnoredOverride, SettingExplanation, explain, text};
use crate::config::runtime::RuntimeConfig;
use saya_config::{
    CliOverrides, ConfigFile, ConnectionsFile, ResolutionInput, SettingSource, resolve,
};

const CONNECTIONS: &str = "[profiles.local]\ntype = 'duckdb'\npath = ':memory:'\n";

/// Hand-builds a `RuntimeConfig` the way the loader does: `resolve` runs the
/// real merge funnel, so the provenance recorded here is the one the CLI
/// would display. `config_path` mirrors the loader's selection (the project
/// file when it exists, `--config` when passed).
fn runtime_from(
    user_toml: &str,
    project_toml: &str,
    cli: CliOverrides,
    process_env: BTreeMap<String, String>,
    config_path: Option<PathBuf>,
) -> RuntimeConfig {
    let connections = ConnectionsFile::from_toml(CONNECTIONS).unwrap();
    let resolved = resolve(
        ResolutionInput::new(connections.clone())
            .with_user(ConfigFile::from_toml(user_toml).unwrap())
            .with_project(ConfigFile::from_toml(project_toml).unwrap())
            .with_process_env(process_env.clone())
            .with_cli(cli),
    )
    .unwrap();
    RuntimeConfig {
        resolved,
        connections,
        config_path,
        connections_path: None,
        cache_scope: PathBuf::from("/tmp/saya-explain-test"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        // The loader seeds secret_values from the process environment; the
        // fixture mirrors that, so a leak of an env-sourced VALUE is visible
        // to the sentinel test below.
        secret_values: process_env,
    }
}

fn entry<'a>(explanation: &'a ConfigExplanation, key: &str) -> &'a SettingExplanation {
    explanation
        .settings
        .iter()
        .find(|setting| setting.key == key)
        .unwrap_or_else(|| panic!("no `{key}` setting in {explanation:?}"))
}

// --- an empty configuration is all defaults ---

include!("explain_tests/defaults_and_keys.rs");

// --- file layers name the file a user would edit ---

include!("explain_tests/source_attribution.rs");

// --- a secret reference shows only its redacted label ---

include!("explain_tests/redaction_and_shape.rs");

// --- the runner rows: distinct display labels over one grouped origin ---

include!("explain_tests/runner_rows.rs");

// --- the text rendering ---

include!("explain_tests/text_rendering.rs");
