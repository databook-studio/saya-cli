use std::collections::BTreeMap;

use saya_types::{DatabaseProfile, SecretRef};
use serde::Deserialize;

use crate::{
    AiProvider, ColorChoice, ConfigError, MemoryMode, OutputFormat, RedactedDiagnostics,
    ThemeChoice,
};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    pub default_profile: Option<String>,
    #[serde(default)]
    pub ai: AiFile,
    #[serde(default)]
    pub run: RunFile,
    #[serde(default)]
    pub jobs: JobsFile,
    #[serde(default)]
    pub output: OutputFile,
    #[serde(default)]
    pub memory: MemoryFile,
    #[serde(default)]
    pub ui: UiFile,
}

impl ConfigFile {
    pub fn from_toml(value: &str) -> Result<Self, ConfigError> {
        toml::from_str(value).map_err(|error| {
            inline_secret_hint(value)
                .map(ConfigError::Parse)
                .unwrap_or_else(|| ConfigError::Parse(error.to_string()))
        })
    }

    pub fn redacted_diagnostics(&self) -> RedactedDiagnostics {
        RedactedDiagnostics::from_file(self)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionsFile {
    #[serde(default)]
    pub profiles: BTreeMap<String, DatabaseProfile>,
}

impl ConnectionsFile {
    pub fn from_toml(value: &str) -> Result<Self, ConfigError> {
        // `DatabaseProfile` carries `#[serde(deny_unknown_fields)]`, so an
        // unknown per-profile key (e.g. a typo'd `sslmodee`) is rejected here
        // by serde itself, against the type's own field set — no shadow list.
        toml::from_str(value).map_err(|error| {
            inline_secret_hint(value)
                .map(ConfigError::Parse)
                .unwrap_or_else(|| ConfigError::Parse(error.to_string()))
        })
    }
}

/// Secret-bearing keys that must hold a *reference* (`{ env =... }`), never
/// an inline value. A plain string here is the most common config mistake and
/// serde's untagged-enum error for it is undiagnosable — replace it with the
/// field, the location, and the fix.
const SECRET_KEYS: &[&str] = &["password", "ssl_ca", "api_key", "private_key", "passphrase"];

fn inline_secret_hint(raw: &str) -> Option<String> {
    let value: toml::Value = toml::from_str(raw).ok()?;
    let mut hits = Vec::new();
    for (section, item) in value.as_table()?.iter() {
        let Some(item) = item.as_table() else {
            continue;
        };
        for (name, val) in item {
            if SECRET_KEYS.contains(&name.as_str()) && val.is_str() {
                hits.push(format!("{section}.{name}"));
            }
            // Nested tables ([profiles.<name>]) hold per-profile secrets.
            if let Some(nested) = val.as_table() {
                for (key, val) in nested {
                    if SECRET_KEYS.contains(&key.as_str()) && val.is_str() {
                        hits.push(format!("{section} `{name}`: {key}"));
                    }
                }
            }
            // Arrays of tables ([[ai.endpoints]]) hold per-entry secrets.
            // The location must name *which* entry — an endpoint's `name`
            // when present, its index otherwise — so the fix points at the
            // right endpoint, not at the section as a whole.
            if let Some(array) = val.as_array() {
                for (index, element) in array.iter().enumerate() {
                    let Some(entry) = element.as_table() else {
                        continue;
                    };
                    for (key, val) in entry {
                        if SECRET_KEYS.contains(&key.as_str()) && val.is_str() {
                            let which = entry
                                .get("name")
                                .and_then(toml::Value::as_str)
                                .map(|entry| format!("[{entry:?}]"))
                                .unwrap_or_else(|| format!("[{index}]"));
                            hits.push(format!("{section}.{name}{which}.{key}"));
                        }
                    }
                }
            }
        }
    }
    if hits.is_empty() {
        return None;
    }
    let locations = hits.join(", ");
    Some(format!(
        "secrets are not allowed inline ({locations}): use a reference like \
         {{ env = \"SAYA_VAR\" }}, {{ file = \"...\" }}, or {{ keyring = \"...\" }}"
    ))
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiFile {
    pub provider: Option<AiProvider>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub allow_data_sharing: Option<bool>,
    pub api_key: Option<SecretRef>,
    pub temperature: Option<f32>,
    /// Total budget for establishing a request or a non-streaming response.
    pub timeout_seconds: Option<u64>,
    /// Maximum silence between stream chunks before the provider is stalled.
    pub idle_timeout_seconds: Option<u64>,
    /// Per-response output-token ceiling requested from the provider.
    pub max_output_tokens: Option<u32>,
    /// Provider retry backoff in milliseconds, tried in order before the
    /// provider gives up. Absent keeps the default three-entry schedule. An
    /// empty list means "do not retry" (one attempt, no sleeps). The list
    /// length is bounded at resolve time.
    pub retry_delays_ms: Option<Vec<u64>>,
    /// Ceiling on the approximate byte size of the conversation the agent loop
    /// assembles and sends to the provider. The loop trims under it (oldest
    /// tool results dropped, newest truncated with a marker) rather than abort.
    pub context_byte_budget: Option<usize>,
    /// The model's context window in tokens, as the user declares it. This is
    /// how a model the built-in table does not know — a private gateway serving
    /// a name of its own — gets a window at all. `None` defers to the table.
    pub context_window_tokens: Option<u64>,
    /// Show the model's chain-of-thought in the transcript. Off by default:
    /// thinking is verbose (measured at ~2x the answer length) and restates
    /// database contents in prose, so a user who did not ask for it must not
    /// get it. Display only — reasoning is never persisted regardless of this
    /// setting.
    pub show_thinking: Option<bool>,
    /// Named endpoints a run's roles can bind to (`[[ai.endpoints]]`). Empty
    /// by default: an absent section changes nothing for anyone, and every
    /// role keeps the plain `[ai]` block through the `orchestrator` fallback
    /// at resolution.
    #[serde(default)]
    pub endpoints: Vec<EndpointFile>,
}

/// One `[[ai.endpoints]]` entry: a named endpoint a run's role can bind to.
///
/// The name is required — it is the key roles bind to. Unset fields inherit
/// the plain `[ai]` block at resolution, so an endpoint only overrides what
/// it declares (typically `base_url` and `api_key`); `name` never inherits.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointFile {
    /// The run-scoped endpoint name. The same shape the run contracts use,
    /// validated at resolution against `saya_types::is_name_shaped`.
    pub name: String,
    pub provider: Option<AiProvider>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    /// A reference (`{ env = ... }`), never an inline value — `SecretRef` is
    /// an untagged enum, so an inline string fails to parse with the
    /// `inline_secret_hint` diagnostic naming this endpoint.
    pub api_key: Option<SecretRef>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunFile {
    pub read_only: Option<bool>,
    pub max_rows: Option<usize>,
    pub max_iterations: Option<usize>,
    /// Independent agent attempts per question. The resolved default is `1`,
    /// which is today's single-run behaviour — a user who sets nothing changes
    /// nothing. Each additional candidate is another full agent run (model
    /// calls and database queries), so this multiplies cost roughly linearly.
    /// Bounded at resolve time; the selection logic that consumes it is a
    /// separate task and nothing reads this yet.
    pub candidates: Option<usize>,
    pub query_timeout_seconds: Option<u64>,
}

/// The `[jobs]` section: the default budgets a *run* is declared with when
/// the run's specification and each of its steps declare none. `ConfigFile`
/// is `deny_unknown_fields`, so this section must be declared here before
/// any config may carry it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobsFile {
    /// Default wall-clock ceiling for a run, in seconds. Absent: a run has
    /// no wall-clock ceiling until the run itself declares one.
    pub wall_clock_seconds: Option<u64>,
    /// Default token ceilings keyed by run-scoped endpoint name — the same
    /// per-endpoint shape the run contracts carry. Absent or empty: no
    /// endpoint has a token ceiling.
    pub tokens_per_endpoint: Option<BTreeMap<String, u64>>,
    /// Default turn ceiling for a run's episodes. Absent falls back to
    /// `[run] max_iterations` — see the resolution.
    pub turns: Option<u64>,
    /// Default ceiling on total tool calls across a run's episodes. Absent:
    /// no ceiling.
    pub tool_calls: Option<u64>,
    /// Default download budgets for the `http_download` tool, as the
    /// `[jobs.fetch]` sub-table (M3-3): per-file bytes, total run download
    /// bytes, and the per-request timeout. Absent: the harness's
    /// conservative defaults. There is deliberately no environment override
    /// for any of it (plan G3).
    pub fetch: Option<FetchJobsFile>,
    /// The `[jobs.runner]` sub-table (M5-4): the universe of programs a run's
    /// approved runner scope may name, and the default per-process timeout.
    /// Absent: no runner programs are approved and the conservative timeout
    /// default applies. There is deliberately no environment override for
    /// any of it (plan G3).
    pub runner: Option<RunnerJobsFile>,
}

/// The `[jobs.runner]` sub-table: the runner programs a run's approved
/// runner scope may draw from and the default wall-clock ceiling for one
/// child process (M5-4). Each key is optional: an absent `allow` approves no
/// programs (the runner capability is absent entirely — there is no default
/// program universe a run gets for free), and an absent `timeout_seconds`
/// resolves to the conservative default. Entries are bare program names in
/// the run-scoped name shape; shells and interpreters are refused at resolve
/// time — a program the runner will never honour must not look approved.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerJobsFile {
    /// The programs a run's runner scope may name. Absent or empty: the run
    /// has no runner capability. Each entry is validated against the same
    /// name shape the run contracts carry, bounded like every set-valued
    /// approval surface, and refused when it names a shell or interpreter.
    pub allow: Option<Vec<String>>,
    /// Default wall-clock ceiling for one child process, in seconds. A
    /// declared zero is a typed resolve error, never a silent clamp — a
    /// zero-second timeout would kill every child before its first byte, a
    /// typo, not an intent. Provisional until M5 measures real runs (U8).
    pub timeout_seconds: Option<u64>,
}

/// The `[jobs.fetch]` sub-table: the download budgets a run's
/// `http_download` calls spend from. Each key is optional and resolved with
/// a conservative default; a declared zero is a typed resolve error, never a
/// silent clamp (the same discipline as the sibling `[jobs]` budgets — a
/// zero here would pause every download before its first byte, a typo, not
/// an intent).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FetchJobsFile {
    /// Per-file download ceiling, in bytes. Absent: the conservative
    /// default. Provisional until M5 measures real runs (U8).
    pub max_file_bytes: Option<u64>,
    /// Ceiling on total download bytes across the whole run. Absent: the
    /// conservative default. Provisional until M5 measures real runs (U8).
    pub max_run_bytes: Option<u64>,
    /// Wall-clock budget for one download request, in seconds. With
    /// resumable partials a longer transfer is a sequence of budgeted
    /// attempts. Absent: the conservative default. Provisional until M5
    /// measures real runs (U8).
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputFile {
    pub format: Option<OutputFormat>,
    pub color: Option<ColorChoice>,
}

/// The `[ui]` section: presentation settings that affect how the TUI paints,
/// not what it does. `theme` selects the colour palette.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiFile {
    pub theme: Option<ThemeChoice>,
}

/// The `[memory]` section.
///
/// Uses `#[serde(deny_unknown_fields)]` so obsolete multi-axis configurations
/// (such as `recall` or `learning`) fail loudly at parse time instead of silently
/// falling back to defaults.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryFile {
    pub mode: Option<MemoryMode>,
    pub max_contracts: Option<u32>,
    pub max_claims_per_contract: Option<u32>,
    pub max_context_bytes: Option<u32>,
}
