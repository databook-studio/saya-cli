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

/// How learned context reaches the prompt. Defaults to `Confirmed` so an upgrade
/// changes nothing until the user opts in (ADR 0002, plan §10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[non_exhaustive]
#[serde(rename_all = "kebab-case")]
pub enum MemoryRecall {
    /// Never inject learned context.
    Off,
    /// Inject only confirmed claims.
    #[default]
    Confirmed,
    /// Confirmed claims plus unconfirmed candidates, labelled as such.
    IncludeCandidates,
}

/// Whether SAYA learns from a turn, and what happens to a proposal. Defaults to
/// `Off` so an upgrade changes nothing until the user opts in (ADR 0002, plan §10).
///
/// Plan §17.4 deferred whether `Suggest`/`AutoCandidate` makes a second provider
/// call to extract candidates or relies on in-band proposals. Decision: in-band.
/// `contract_propose` is already a tool the model calls during the turn, so
/// `Suggest` and `AutoCandidate` differ only in what happens to a proposal — shown,
/// or shown and persisted — not in how it is produced. A second call would add a
/// per-turn cost and latency the plan's own risk register flags, for a capability
/// the tool already provides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[non_exhaustive]
#[serde(rename_all = "kebab-case")]
pub enum MemoryLearning {
    /// No learning; nothing is proposed or persisted.
    #[default]
    Off,
    /// Propose candidates for review; do not persist them automatically.
    Suggest,
    /// Propose candidates and persist them into the review queue.
    AutoCandidate,
}
