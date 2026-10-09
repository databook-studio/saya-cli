//! `[jobs]` — the default budgets a run is declared with when the run's
//! spec and each of its steps declare none.
//!
//! Every ceiling is opt-in: a ceiling left unset is unlimited at the
//! contract level, matching the run contract's own wording. The engine never
//! reads the environment for budgets (plan G3), so no `[jobs]` key has an
//! environment override.

use std::collections::BTreeMap;
use std::time::Duration;

use saya_config::{ConfigError, ConfigFile, ConnectionsFile, ResolutionInput, resolve};

fn resolve_with_user(toml: &str) -> saya_config::ResolvedConfig {
    resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml(toml).expect("fixture must parse")),
    )
    .expect("resolution succeeds")
}

fn runner_program_dir() -> &'static str {
    if cfg!(windows) {
        r"C:\saya-programs"
    } else {
        "/opt/saya-programs"
    }
}

fn toml_string(value: &str) -> String {
    format!("{value:?}")
}

#[path = "jobs/budget_contract.rs"]
mod budget_contract;
#[path = "jobs/budgets.rs"]
mod budgets;
#[path = "jobs/diagnostics.rs"]
mod diagnostics;
#[path = "jobs/fetch.rs"]
mod fetch;
#[path = "jobs/host_commands.rs"]
mod host_commands;
#[path = "jobs/interpreter.rs"]
mod interpreter;
#[path = "jobs/project_layer.rs"]
mod project_layer;
#[path = "jobs/runner_program_dir.rs"]
mod runner_program_dir;
#[path = "jobs/runner_settings.rs"]
mod runner_settings;
