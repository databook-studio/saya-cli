use std::collections::BTreeMap;

use saya_types::{DatabaseProfile, SecretRef};
use serde::{Deserialize, Serialize};

use crate::{ConfigError, RedactedDiagnostics};

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ConfigFile {
    pub default_profile: Option<String>,
    #[serde(default)]
    pub ai: AiFile,
    #[serde(default)]
    pub run: RunFile,
    #[serde(default)]
    pub output: OutputFile,
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
pub struct ConnectionsFile {
    #[serde(default)]
    pub profiles: BTreeMap<String, DatabaseProfile>,
}

impl ConnectionsFile {
    pub fn from_toml(value: &str) -> Result<Self, ConfigError> {
        toml::from_str(value).map_err(|error| ConfigError::Parse(error.to_string()))
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AiFile {
    pub provider: Option<AiProvider>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub allow_data_sharing: Option<bool>,
    pub api_key: Option<SecretRef>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RunFile {
    pub read_only: Option<bool>,
    pub max_rows: Option<usize>,
    pub max_iterations: Option<usize>,
    pub query_timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct OutputFile {
    pub format: Option<OutputFormat>,
    pub color: Option<ColorChoice>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiProvider {
    Ollama,
    OpenaiCompatible,
    Openai,
    Anthropic,
    Gemini,
}

impl AiProvider {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ollama => "ollama",
            Self::OpenaiCompatible => "openai_compatible",
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    Text,
    Json,
    Ndjson,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}
