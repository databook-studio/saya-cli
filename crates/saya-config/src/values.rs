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
