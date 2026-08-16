//! What a turn is configured with, and what can go wrong assembling one.
//!
//! Split out of `runtime.rs` so that module holds only the turn flow itself.
//! These four items are read from several places (`privacy_tests.rs` alone
//! references them seven times), so `runtime` re-exports them and every existing
//! path keeps working.

use saya_config::{AiProvider, ResolvedAi};
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum AgentRuntimeError {
    #[error("{0}")]
    Provider(String),
    #[error("{0}")]
    Database(String),
    #[error("{0}")]
    Agent(String),
    #[error("{0}")]
    Configuration(String),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PromptOverrides {
    pub(crate) provider: Option<AiProvider>,
    pub(crate) model: Option<String>,
    pub(crate) allow_data_sharing: Option<bool>,
    pub(crate) profile: Option<String>,
    pub(crate) included_profiles: Vec<String>,
}

/// Whether result rows may reach the provider. Ollama runs locally, so data
/// never leaves the machine and sharing is implicit; every cloud provider is
/// gated on the user's explicit `allow_data_sharing`.
pub(crate) fn query_data_allowed(provider: AiProvider, allow_data_sharing: bool) -> bool {
    match provider {
        AiProvider::Openai
        | AiProvider::OpenaiCompatible
        | AiProvider::Anthropic
        | AiProvider::Gemini => allow_data_sharing,
        AiProvider::Ollama => true,
    }
}

/// The AI settings this turn actually runs with, after per-prompt overrides.
///
/// Changing provider clears `base_url`: a URL configured for one provider is
/// meaningless — and possibly wrong in a way that silently reaches the wrong
/// host — when pointed at another.
pub(crate) fn effective_ai(base: &ResolvedAi, overrides: &PromptOverrides) -> ResolvedAi {
    let mut ai = base.clone();
    if let Some(provider) = overrides.provider {
        if ai.provider != provider {
            ai.base_url = None;
        }
        ai.provider = provider;
    }
    if let Some(model) = overrides.model.as_ref() {
        ai.model = model.clone();
    }
    if let Some(value) = overrides.allow_data_sharing {
        ai.allow_data_sharing = value;
    }
    ai
}
