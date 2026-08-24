use std::collections::BTreeMap;

use saya_types::{DatabaseProfile, SecretRef};
use serde::Deserialize;

use crate::{AiProvider, ColorChoice, ConfigError, MemoryMode, OutputFormat, RedactedDiagnostics};

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
}

impl ConfigFile {
    pub fn from_toml(value: &str) -> Result<Self, ConfigError> {
        toml::from_str(value).map_err(|error| ConfigError::Parse(error.to_string()))
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
        let file: Self =
            toml::from_str(value).map_err(|error| ConfigError::Parse(error.to_string()))?;
        // `deny_unknown_fields` cannot be combined with the internally tagged
        // `DatabaseProfile` enum, so per-profile keys are validated against
        // the raw table here. This is what makes a typo'd `sslmodee` fail at
        // parse time instead of silently downgrading TLS.
        let raw: toml::Value =
            toml::from_str(value).map_err(|error| ConfigError::Parse(error.to_string()))?;
        if let Some(profiles) = raw.get("profiles").and_then(toml::Value::as_table) {
            for (name, profile) in profiles {
                validate_profile_keys(name, profile)?;
            }
        }
        Ok(file)
    }
}

const POSTGRES_PROFILE_KEYS: &[&str] = &[
    "type", "host", "port", "database", "user", "sslmode", "ssl_mode", "password",
];
const MYSQL_PROFILE_KEYS: &[&str] = &[
    "type", "host", "port", "database", "user", "sslmode", "ssl_mode", "ssl_ca", "password",
];
const FILE_DUCKDB_KEYS: &[&str] = &["type", "path", "read_only"];
const SQLITE_PROFILE_KEYS: &[&str] = FILE_DUCKDB_KEYS;
const SNOWFLAKE_PROFILE_KEYS: &[&str] = &[
    "type",
    "account",
    "user",
    "auth_type",
    "private_key",
    "password",
    "passphrase",
    "warehouse",
    "database",
    "schema",
    "role",
];

fn validate_profile_keys(name: &str, profile: &toml::Value) -> Result<(), ConfigError> {
    let Some(table) = profile.as_table() else {
        return Ok(());
    };
    let backend = table
        .get("type")
        .and_then(toml::Value::as_str)
        .unwrap_or_default();
    let allowed: &[&str] = match backend {
        "postgresql" => POSTGRES_PROFILE_KEYS,
        "mysql" => MYSQL_PROFILE_KEYS,
        "duckdb" => FILE_DUCKDB_KEYS,
        "sqlite" => SQLITE_PROFILE_KEYS,
        "snowflake" => SNOWFLAKE_PROFILE_KEYS,
        _ => return Ok(()),
    };
    for key in table.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(ConfigError::Parse(format!(
                "unknown key `{key}` in profile `{name}` ({backend})"
            )));
        }
    }
    Ok(())
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
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunFile {
    pub read_only: Option<bool>,
    pub max_rows: Option<usize>,
    pub max_iterations: Option<usize>,
    pub query_timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputFile {
    pub format: Option<OutputFormat>,
    pub color: Option<ColorChoice>,
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
