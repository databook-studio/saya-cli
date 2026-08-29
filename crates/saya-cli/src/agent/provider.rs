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
        .with_max_output_tokens(config.max_output_tokens);
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
