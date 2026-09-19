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
    /// The session's endpoint, when it has been bound. `endpoint_bound` keeps
    /// an explicit clear distinct from an unset one-shot override.
    pub(crate) endpoint: Option<String>,
    pub(crate) endpoint_bound: bool,
}

/// Whether result rows may reach the provider when no endpoint URL is known.
/// This preserves the historical default for Ollama; callers with a resolved
/// endpoint should use [`query_data_allowed_for_endpoint`].
pub(crate) fn query_data_allowed(provider: AiProvider, allow_data_sharing: bool) -> bool {
    query_data_allowed_for_endpoint(provider, None, allow_data_sharing)
}

/// Whether result rows may reach the configured provider endpoint.
///
/// Ollama is local only when its endpoint is absent (the provider's own
/// localhost default) or unambiguously points at loopback. A malformed URL,
/// credentials, query, or fragment is not enough evidence to send database
/// values, so it fails closed. Explicit sharing always wins.
pub(crate) fn query_data_allowed_for_endpoint(
    provider: AiProvider,
    base_url: Option<&str>,
    allow_data_sharing: bool,
) -> bool {
    allow_data_sharing || is_local_endpoint(provider, base_url)
}

pub(crate) fn is_local_endpoint(provider: AiProvider, base_url: Option<&str>) -> bool {
    match provider {
        AiProvider::Ollama => base_url.is_none_or(|value| {
            let Ok(url) = url::Url::parse(value) else {
                return false;
            };
            if url.username() != ""
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return false;
            }
            let Some(host) = url.host_str() else {
                return false;
            };
            host.eq_ignore_ascii_case("localhost")
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|address| address.is_loopback())
        }),
        AiProvider::Openai
        | AiProvider::OpenaiCompatible
        | AiProvider::Anthropic
        | AiProvider::Gemini => false,
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
    if overrides.endpoint_bound {
        ai.base_url = overrides.endpoint.clone();
    }
    if let Some(model) = overrides.model.as_ref() {
        ai.model = model.clone();
    }
    if let Some(value) = overrides.allow_data_sharing {
        ai.allow_data_sharing = value;
    }
    ai
}
