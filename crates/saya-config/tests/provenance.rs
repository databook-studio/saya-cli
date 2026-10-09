//! Setting provenance: which layer supplied each user-facing setting.
//!
//! Provenance is a value-free observation of the existing merge. It names
//! the winning layer and — for a secret — only the redacted reference
//! label, never a value or a path, and it never changes resolution: the
//! same values, the same errors, the same `ignored_project_overrides`.

#[path = "provenance/endpoint_fields.rs"]
mod endpoint_fields;

#[path = "provenance/ignored_overrides.rs"]
mod ignored_overrides;

#[path = "provenance/precedence.rs"]
mod precedence;

#[path = "provenance/profile.rs"]
mod profile;

#[path = "provenance/secrecy.rs"]
mod secrecy;

use saya_config::{ConnectionsFile, ResolutionInput, SettingSource};

fn empty() -> ResolutionInput {
    ResolutionInput::new(ConnectionsFile::default())
}

fn source_of(resolved: &saya_config::ResolvedConfig, key: &str) -> SettingSource {
    resolved
        .provenance
        .get(key)
        .unwrap_or_else(|| panic!("no provenance recorded for {key}"))
        .source
        .clone()
}
