use saya_agent::{
    AnthropicProvider, ChatProvider, GeminiProvider, OllamaProvider, OpenAiCompatibleProvider,
    ProviderError, ProviderSettings,
};
use saya_config::{AiProvider, ResolvedAi, SecretResolver};

pub(crate) fn build(
    config: &ResolvedAi,
    resolver: &dyn SecretResolver,
) -> Result<Box<dyn ChatProvider>, ProviderError> {
    let settings = ProviderSettings::new(config.model.clone(), config.base_url.clone())
        .with_temperature(config.temperature)
        .with_timeout(std::time::Duration::from_secs(config.timeout_seconds))
        .with_idle_timeout(std::time::Duration::from_secs(config.idle_timeout_seconds))
        .with_request_timeout(std::time::Duration::from_secs(
            config.request_timeout_seconds,
        ))
        .with_max_output_tokens(config.max_output_tokens)
        .with_reasoning_effort(config.reasoning_effort.and_then(effort_for))
        .with_app_attribution(config.app_attribution)
        .with_retry_delays(
            config
                .retry_delays_ms
                .iter()
                .map(|ms| std::time::Duration::from_millis(*ms))
                .collect(),
        );
    match config.provider {
        AiProvider::Ollama => Ok(Box::new(OllamaProvider::new(settings)?)),
        AiProvider::Openai | AiProvider::OpenaiCompatible => {
            let key = config
                .api_key
                .as_ref()
                .map(|reference| resolver.resolve(reference))
                .transpose()
                .map_err(|_| ProviderError::configuration("API key unavailable"))?;
            Ok(Box::new(OpenAiCompatibleProvider::new(
                settings,
                key.as_ref().map(|value| value.expose()),
            )?))
        }
        AiProvider::Anthropic => {
            let key = config
                .api_key
                .as_ref()
                .map(|reference| resolver.resolve(reference))
                .transpose()
                .map_err(|_| ProviderError::configuration("API key unavailable"))?;
            Ok(Box::new(AnthropicProvider::new(
                settings,
                key.as_ref().map(|value| value.expose()),
            )?))
        }
        AiProvider::Gemini => {
            let key = config
                .api_key
                .as_ref()
                .map(|reference| resolver.resolve(reference))
                .transpose()
                .map_err(|_| ProviderError::configuration("API key unavailable"))?;
            Ok(Box::new(GeminiProvider::new(
                settings,
                key.as_ref().map(|value| value.expose()),
            )?))
        }
    }
}

/// The config spelling of an effort, as the agent's provider-neutral intent.
/// Two types rather than one because the layers point the other way: the
/// config crate must not depend on the agent crate, and the *settable* set is
/// narrower than the wire set (no `default`, no `minimal` — see
/// [`saya_config::ReasoningEffort`]). `adaptive` maps to `None`: no pinned
/// effort, so the loop's per-turn policy decides, exactly as when the key is
/// unset.
fn effort_for(effort: saya_config::ReasoningEffort) -> Option<saya_agent::ReasoningEffort> {
    match effort {
        saya_config::ReasoningEffort::Adaptive => None,
        saya_config::ReasoningEffort::Low => Some(saya_agent::ReasoningEffort::Low),
        saya_config::ReasoningEffort::Medium => Some(saya_agent::ReasoningEffort::Medium),
        saya_config::ReasoningEffort::High => Some(saya_agent::ReasoningEffort::High),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use saya_config::{ConfigFile, ConnectionsFile, MapSecretResolver, ResolutionInput};
    use std::time::Duration;

    /// The retry schedule a provider built from `[ai]` config hands the loop.
    fn schedule_for(config: &str) -> Vec<Duration> {
        let input = ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml(config).unwrap());
        let ai = saya_config::resolve(input).unwrap().ai;
        build(&ai, &MapSecretResolver::new(Vec::new()))
            .unwrap()
            .retry_delays()
    }

    #[test]
    fn configured_retry_delays_reach_the_built_provider_in_milliseconds() {
        assert_eq!(
            schedule_for("[ai]\nprovider = \"ollama\"\nretry_delays_ms = [100, 200, 300]\n"),
            vec![
                Duration::from_millis(100),
                Duration::from_millis(200),
                Duration::from_millis(300),
            ]
        );
    }

    #[test]
    fn an_unset_retry_schedule_builds_the_default_backoff() {
        assert_eq!(
            schedule_for("[ai]\nprovider = \"ollama\"\n"),
            vec![
                Duration::from_millis(500),
                Duration::from_millis(1000),
                Duration::from_millis(2000),
                Duration::from_millis(4000),
                Duration::from_millis(8000),
                Duration::from_millis(16000),
                Duration::from_millis(30000),
                Duration::from_millis(30000),
            ]
        );
    }
}
