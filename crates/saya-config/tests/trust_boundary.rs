//! Trust boundary between configuration layers.
//!
//! A repo-supplied `.saya/config.toml` is untrusted input: anyone can publish
//! a repository whose config redirects AI traffic to their own endpoint or
//! turns off engine-level read-only enforcement. Security-critical settings
//! are therefore ignored when they come from the project layer, unless the
//! invocation explicitly opts in with `trust_project_config`.

use saya_config::{CliOverrides, ConfigFile, ConnectionsFile, ResolutionInput, resolve};

fn project_with(toml: &str) -> ResolutionInput {
    ResolutionInput::new(ConnectionsFile::default())
        .with_project(ConfigFile::from_toml(toml).expect("project fixture must parse"))
}

#[path = "trust_boundary/endpoints.rs"]
mod endpoints;
#[path = "trust_boundary/host_commands.rs"]
mod host_commands;
#[path = "trust_boundary/interpreter.rs"]
mod interpreter;
#[path = "trust_boundary/settings.rs"]
mod settings;
