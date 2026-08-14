//! The headless `saya preferences` adapter: resolves arguments, calls the
//! `PreferenceStore` surface, and renders typed results. No policy lives here
//! — scope rules, validation and admission all live in `saya-types` and
//! `saya-store`. This layer resolves a profile, builds the typed value, calls
//! the store, and renders. Mirrors `commands/contracts.rs`.
//!
//! Dispatch and shared helpers live here; the read command (`list`) is in
//! `preferences_read.rs`, the write commands (`set`/`unset`) in
//! `preferences_write.rs`, value building in `preferences_args.rs`, and the
//! value→DTO mapping in `preferences_map.rs`.

mod preferences_args;
mod preferences_map;
mod preferences_read;
mod preferences_write;

use super::contracts::resolve_profile;
use crate::cli::PreferencesCommand;
use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;
use saya_store::SqliteStateStore;
use saya_types::{PreferenceScope, ScopeRequirement};

use preferences_args::PrefArgError;

/// Exit code for a typed preferences failure, matching `contracts`' ad-hoc
/// per-command scheme.
pub(super) const EXIT_PREF_ERROR: i32 = 2;
pub(super) const STORE_UNAVAILABLE_MSG: &str =
    "Local state store unavailable; preferences could not be read.";

pub async fn run_preferences(
    command: PreferencesCommand,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    store: &SqliteStateStore,
) -> Result<i32, Box<dyn std::error::Error>> {
    match command {
        PreferencesCommand::List { profile } => {
            preferences_read::list(store, runtime, format, profile).await
        }
        PreferencesCommand::Set {
            kind,
            value,
            profile,
        } => preferences_write::set(store, runtime, format, kind, &value, profile).await,
        PreferencesCommand::Unset { kind, profile } => {
            preferences_write::unset(store, runtime, format, kind, profile).await
        }
    }
}

/// Resolves the scope a kind requires. A profile-scoped kind needs a profile
/// (`--profile` or the active one); a global kind rejects `--profile`. The error
/// names which — never the value.
pub(super) fn resolve_scope(
    runtime: &RuntimeConfig,
    required: ScopeRequirement,
    profile: Option<String>,
) -> Result<(PreferenceScope, String), (i32, String)> {
    match required {
        ScopeRequirement::Global => {
            if profile.is_some() {
                return Err(scope_conflict("global"));
            }
            Ok((PreferenceScope::Global, "global".into()))
        }
        ScopeRequirement::Profile => {
            let (name, identity) = resolve_profile(runtime, profile.as_deref())?;
            Ok((PreferenceScope::Profile(identity), name))
        }
    }
}

fn scope_conflict(required: &'static str) -> (i32, String) {
    (
        EXIT_PREF_ERROR,
        PrefArgError::ScopeConflict(required).to_string(),
    )
}
