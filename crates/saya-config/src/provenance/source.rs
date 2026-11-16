//! The provenance data model: which layer won, one setting's winning
//! origin, and the map of origins keyed by dotted config key.

use std::collections::BTreeMap;

use serde::Serialize;

/// Which layer won for one setting. Value-free: `Env`/`EnvFile` name the
/// environment *variable*, `Cli` the *flag* text; the file layers stay
/// anonymous — the CLI already knows the paths it loaded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "layer", rename_all = "snake_case")]
pub enum SettingSource {
    Default,
    /// No layer stated the setting; its value was derived from another
    /// resolved fact (the model's context window) rather than a built-in
    /// constant. Today only `ai.context_byte_budget` resolves this way.
    WindowDerived,
    UserFile,
    ProjectFile,
    EnvFile {
        var: String,
    },
    Env {
        var: String,
    },
    Cli {
        flag: String,
    },
}

/// One setting's winning origin; `reference` is the redacted label of a
/// winning secret reference (`env:NAME`, `file:[redacted path]`), never a
/// value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SettingOrigin {
    pub source: SettingSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

/// The user-facing settings' winning layers, keyed by dotted config key
/// (`ai.model`). Serializes as a plain map.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SettingProvenance(pub(super) BTreeMap<String, SettingOrigin>);

impl SettingProvenance {
    pub fn get(&self, key: &str) -> Option<&SettingOrigin> {
        self.0.get(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &SettingOrigin)> {
        self.0.iter()
    }
}
