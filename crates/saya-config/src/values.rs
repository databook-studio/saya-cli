use serde::{Deserialize, Serialize};

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

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ollama" => Some(Self::Ollama),
            "openai_compatible" => Some(Self::OpenaiCompatible),
            "openai" => Some(Self::Openai),
            "anthropic" => Some(Self::Anthropic),
            "gemini" => Some(Self::Gemini),
            _ => None,
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

impl OutputFormat {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "text" => Some(Self::Text),
            "json" => Some(Self::Json),
            "ndjson" => Some(Self::Ndjson),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}

/// Which colour palette the TUI paints with.
///
/// `Auto` resolves at session start: it honours `COLORFGBG` when the terminal
/// publishes a background, and falls back to `Dark` otherwise. `Dark` and
/// `Light` force a palette regardless of the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThemeChoice {
    #[default]
    Auto,
    Dark,
    Light,
}

impl ThemeChoice {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "dark" => Some(Self::Dark),
            "light" => Some(Self::Light),
            _ => None,
        }
    }
}

/// How context compaction behaves once the window fills.
///
/// - `Auto`: `/compact` works and a finished turn at or past the compact
///   threshold summarises older turns on its own.
/// - `Manual`: `/compact` works but nothing ever fires on its own.
/// - `Off`: neither the automatic trigger nor the 70% warning fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CompactionMode {
    /// Automatic compaction on crossing the threshold (the default).
    #[default]
    Auto,
    /// Only an explicit `/compact` compacts.
    Manual,
    /// No automatic trigger and no context warning; `/compact` still works
    /// when explicitly asked (it is working-memory maintenance, not a
    /// background behaviour).
    Off,
}

impl CompactionMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Manual => "manual",
            Self::Off => "off",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "manual" => Some(Self::Manual),
            "off" => Some(Self::Off),
            _ => None,
        }
    }
}

/// Memory operational mode.
///
/// - `Off`: memory is completely disabled — no store queries, no proposals, no observation logging.
/// - `Assisted`: explicit user statements are active knowledge, assistant inferences are pending,
///   active knowledge is supplied in recall, and pending knowledge is labelled unconfirmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[non_exhaustive]
#[serde(rename_all = "kebab-case")]
pub enum MemoryMode {
    /// Memory is disabled.
    #[default]
    Off,
    /// Active knowledge is recalled and assistant proposals are persisted as candidates for review.
    Assisted,
}

impl MemoryMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Assisted => "assisted",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "off" => Some(Self::Off),
            "assisted" => Some(Self::Assisted),
            _ => None,
        }
    }
}
