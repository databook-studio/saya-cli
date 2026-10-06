//! Value-free provenance: which layer supplied each user-facing setting.
//!
//! Recorded inside the single merge funnel (`layers.rs`) where a layer
//! assigns — never a parallel merge. A secret appears only as the redacted
//! label of its reference: no values, no paths.

use std::collections::BTreeMap;

use saya_types::SecretRef;
use serde::Serialize;

use crate::{endpoints::ResolvedEndpoint, model::EndpointFile};

/// Which layer won for one setting. Value-free: `Env`/`EnvFile` name the
/// environment *variable*, `Cli` the *flag* text; the file layers stay
/// anonymous — the CLI already knows the paths it loaded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "layer", rename_all = "snake_case")]
pub enum SettingSource {
    Default,
    UserFile,
    ProjectFile,
    EnvFile { var: String },
    Env { var: String },
    Cli { flag: String },
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
pub struct SettingProvenance(BTreeMap<String, SettingOrigin>);

impl SettingProvenance {
    pub fn get(&self, key: &str) -> Option<&SettingOrigin> {
        self.0.get(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &SettingOrigin)> {
        self.0.iter()
    }
}

/// The write side: called from `layers.rs` where a layer assigns, so the
/// recording cannot drift from the merge it observes.
#[derive(Debug, Clone, Default)]
pub(crate) struct Recorder {
    origins: SettingProvenance,
}

impl Recorder {
    pub(crate) fn record(&mut self, key: &str, source: SettingSource) {
        self.record_reference(key, source, None);
    }

    /// Records a secret-bearing key with the reference's redacted label.
    pub(crate) fn record_secret(&mut self, key: &str, source: SettingSource, secret: &SecretRef) {
        self.record_reference(key, source, Some(secret.redacted_label()));
    }

    pub(crate) fn record_reference(
        &mut self,
        key: &str,
        source: SettingSource,
        reference: Option<String>,
    ) {
        self.origins
            .0
            .insert(key.to_owned(), SettingOrigin { source, reference });
    }

    /// The pre-project copy `rollback` restores from.
    pub(crate) fn snapshot(&self) -> SettingProvenance {
        self.origins.clone()
    }

    /// Rolls the keys `revert_untrusted` reported back to the pre-project
    /// snapshot: restored where the snapshot holds the entry, removed where
    /// it does not (a project-only endpoint). A whole-entry endpoint key
    /// (`ai.endpoints["name"]`) rolls its recorded per-field descendants.
    pub(crate) fn rollback(&mut self, keys: &[String], snapshot: &SettingProvenance) {
        for key in keys {
            self.restore(key, snapshot);
            let prefix = format!("{key}.");
            let descendants: Vec<String> = self
                .origins
                .0
                .range(key.clone()..)
                .take_while(|(candidate, _)| candidate.starts_with(&prefix))
                .map(|(candidate, _)| candidate.clone())
                .collect();
            for candidate in descendants {
                self.restore(&candidate, snapshot);
            }
        }
    }

    fn restore(&mut self, key: &str, snapshot: &SettingProvenance) {
        match snapshot.0.get(key) {
            Some(origin) => self.origins.0.insert(key.to_owned(), origin.clone()),
            None => self.origins.0.remove(key),
        };
    }

    pub(crate) fn origin_of(&self, key: &str) -> Option<&SettingOrigin> {
        self.origins.0.get(key)
    }

    pub(crate) fn finish(self) -> SettingProvenance {
        self.origins
    }
}

/// Records the `[[ai.endpoints]]` fields a layer supplied: an overlay
/// records exactly the fields it set; a new entry, the fields it declares.
/// Keys use the `ai.endpoints["name"].field` shape `revert_untrusted`
/// reports, so its ignored list alone drives the rollback.
pub(crate) fn record_endpoint_fields(
    recorder: &mut Recorder,
    name: &str,
    fields: &EndpointFile,
    source: SettingSource,
) {
    let base = format!("ai.endpoints[{name:?}]");
    for (field, supplied) in [
        ("provider", fields.provider.is_some()),
        ("model", fields.model.is_some()),
        ("base_url", fields.base_url.is_some()),
    ] {
        if supplied {
            recorder.record(&format!("{base}.{field}"), source.clone());
        }
    }
    if let Some(secret) = fields.api_key.as_ref() {
        recorder.record_secret(&format!("{base}.api_key"), source.clone(), secret);
    }
}

/// The dotted config keys `redacted_diagnostics` reports (the settings
/// `config show`/`doctor` display), plus `instructions.*` and the
/// host/session-command presence keys. The drift-guard test in this module
/// proves every key here is one the merge funnels record.
const USER_FACING: &[&str] = &[
    "profile_name",
    "ai.provider",
    "ai.model",
    "ai.base_url",
    "ai.api_key",
    "ai.allow_data_sharing",
    "ai.temperature",
    "ai.timeout_seconds",
    "ai.idle_timeout_seconds",
    "ai.max_output_tokens",
    "ai.retry_delays_ms",
    "ai.context_byte_budget",
    "ai.context_window_tokens",
    "ai.show_thinking",
    "ai.compaction",
    "run.read_only",
    "run.max_rows",
    "run.max_iterations",
    "run.candidates",
    "run.query_timeout_seconds",
    "jobs.wall_clock_seconds",
    "jobs.tokens_per_endpoint",
    "jobs.turns",
    "jobs.tool_calls",
    "jobs.fetch",
    "jobs.runner",
    "output.format",
    "output.color",
    "ui.theme",
    "memory.mode",
    "memory.max_contracts",
    "memory.max_claims_per_contract",
    "memory.max_context_bytes",
    "instructions.project",
    "instructions.personal",
    "instructions.project_max_bytes",
    "instructions.personal_max_bytes",
    "instructions.total_max_bytes",
    "host_commands.pass_env",
    "host_commands.timeout_seconds",
    "session_commands.deny",
];

/// After resolution, every user-facing key with no recorded origin is a
/// built-in default: `resolve` applied it without any layer stating it.
/// Endpoint fields are keyed exactly as `revert_untrusted` names them, so
/// one key shape serves recording, rollback, and the defaults fill.
pub(crate) fn apply_user_facing_defaults(
    provenance: &mut SettingProvenance,
    endpoints: &BTreeMap<String, ResolvedEndpoint>,
) {
    let default = || SettingOrigin {
        source: SettingSource::Default,
        reference: None,
    };
    for key in USER_FACING {
        provenance
            .0
            .entry((*key).to_owned())
            .or_insert_with(default);
    }
    for name in endpoints.keys() {
        for field in ["provider", "model", "base_url", "api_key"] {
            provenance
                .0
                .entry(format!("ai.endpoints[{name:?}].{field}"))
                .or_insert_with(default);
        }
    }
}

/// The `config show` diagnostics fields, each paired with the dotted config
/// key the merge funnels record it under, in the order `--explain` prints.
/// `profile_dialect` has no entry (derived from the selected profile, not a
/// setting) and the `endpoints` map has none (the recorder keys each
/// endpoint's fields individually), so they stay out of the explanation.
const USER_FACING_DIAGNOSTICS: &[(&str, &str)] = &[
    ("profile_name", "profile_name"),
    ("provider", "ai.provider"),
    ("model", "ai.model"),
    ("base_url", "ai.base_url"),
    ("api_key_reference", "ai.api_key"),
    ("allow_data_sharing", "ai.allow_data_sharing"),
    ("temperature", "ai.temperature"),
    ("timeout_seconds", "ai.timeout_seconds"),
    ("idle_timeout_seconds", "ai.idle_timeout_seconds"),
    ("max_output_tokens", "ai.max_output_tokens"),
    ("retry_delays_ms", "ai.retry_delays_ms"),
    ("context_byte_budget", "ai.context_byte_budget"),
    ("context_window_tokens", "ai.context_window_tokens"),
    ("show_thinking", "ai.show_thinking"),
    ("compaction", "ai.compaction"),
    ("read_only", "run.read_only"),
    ("max_rows", "run.max_rows"),
    ("max_iterations", "run.max_iterations"),
    ("candidates", "run.candidates"),
    ("query_timeout_seconds", "run.query_timeout_seconds"),
    ("jobs_wall_clock_seconds", "jobs.wall_clock_seconds"),
    ("jobs_tokens_per_endpoint", "jobs.tokens_per_endpoint"),
    ("jobs_turns", "jobs.turns"),
    ("jobs_tool_calls", "jobs.tool_calls"),
    ("jobs_runner_allow", "jobs.runner"),
    ("jobs_runner_program_dir", "jobs.runner"),
    ("jobs_runner_timeout_seconds", "jobs.runner"),
    ("output_format", "output.format"),
    ("output_color", "output.color"),
    ("ui_theme", "ui.theme"),
    ("memory_mode", "memory.mode"),
    ("memory_max_contracts", "memory.max_contracts"),
    (
        "memory_max_claims_per_contract",
        "memory.max_claims_per_contract",
    ),
    ("memory_max_context_bytes", "memory.max_context_bytes"),
];

/// The diagnostics-field → dotted-key table `config show --explain` renders:
/// which `ResolvedDiagnostics` field each user-facing setting rides on, and
/// the key the merge funnels record it under.
pub fn user_facing_keys() -> &'static [(&'static str, &'static str)] {
    USER_FACING_DIAGNOSTICS
}

#[cfg(test)]
#[path = "provenance_tests.rs"]
mod tests;
