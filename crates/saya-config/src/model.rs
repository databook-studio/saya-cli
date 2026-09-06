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
    /// Show the model's chain-of-thought in the transcript. Off by default:
    /// thinking is verbose (measured at ~2x the answer length) and restates
    /// database contents in prose, so a user who did not ask for it must not
    /// get it. Display only — reasoning is never persisted regardless of this
    /// setting.
    pub show_thinking: Option<bool>,
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
