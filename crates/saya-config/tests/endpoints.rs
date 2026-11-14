//! `[[ai.endpoints]]` — the named AI endpoint pool a run's roles bind to.
//!
//! Resolution: each endpoint is a delta over the plain `[ai]` block (unset
//! fields inherit it), the resolved map is keyed by run-scoped name with
//! duplicates rejected as a typed error, and `orchestrator` always resolves —
//! to the plain `[ai]` block when nothing is declared with that name, so an
//! existing config keeps working untouched.

#[path = "endpoints/bounds.rs"]
mod bounds;

#[path = "endpoints/orchestrator.rs"]
mod orchestrator;

#[path = "endpoints/redaction.rs"]
mod redaction;

#[path = "endpoints/rejection.rs"]
mod rejection;

use saya_config::{ConfigError, ConfigFile, ConnectionsFile, ResolutionInput, resolve};

fn resolve_with_user(toml: &str) -> Result<saya_config::ResolvedConfig, ConfigError> {
    resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml(toml).expect("fixture must parse")),
    )
}
