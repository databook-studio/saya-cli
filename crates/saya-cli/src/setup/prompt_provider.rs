//! The provider questions (S16 invariant 2b): provider, model (default
//! shown, editable), base URL where the provider needs one, and — for
//! anything but ollama — an environment-variable NAME for the API key. The
//! key's value is never asked for, echoed, or stored; only its presence is
//! reported.

use saya_config::AiProvider;

use super::draft::ProviderDraft;
use super::prompt::{Cancel, Prompter, env_field, text_field};

/// What a provider asks beyond its model: ollama is local (default base URL,
/// no key); cloud providers carry a suggested key variable; an
/// openai_compatible endpoint must state its base URL.
#[derive(Clone, Copy)]
enum ProviderAsk {
    Local(&'static str),
    Cloud(&'static str),
    Compatible,
}

const PROVIDER_CHOICES: [(AiProvider, &str, ProviderAsk); 5] = [
    (
        AiProvider::Ollama,
        "qwen2.5-coder:14b",
        ProviderAsk::Local("http://localhost:11434"),
    ),
    (
        AiProvider::Openai,
        "gpt-4o-mini",
        ProviderAsk::Cloud("OPENAI_API_KEY"),
    ),
    (
        AiProvider::OpenaiCompatible,
        "gpt-4o-mini",
        ProviderAsk::Compatible,
    ),
    (
        AiProvider::Anthropic,
        "claude-sonnet-4-5",
        ProviderAsk::Cloud("ANTHROPIC_API_KEY"),
    ),
    (
        AiProvider::Gemini,
        "gemini-2.5-flash",
        ProviderAsk::Cloud("GEMINI_API_KEY"),
    ),
];

pub(crate) fn collect_provider(
    prompter: &mut Prompter<'_>,
) -> Result<Option<ProviderDraft>, Cancel> {
    let mut names: Vec<&str> = PROVIDER_CHOICES.iter().map(|(p, ..)| p.as_str()).collect();
    names.push("skip");
    let choice = prompter.menu("Choose an AI provider:", &names, Some(0))?;
    if choice == PROVIDER_CHOICES.len() {
        return Ok(None); // skip
    }
    let (provider, default_model, ask) = PROVIDER_CHOICES[choice];
    let model = prompter.ask(&format!("Model [{default_model}]: "), |line| {
        text_field(line, Some(default_model))
    })?;
    let (base_url, api_key_env) = match ask {
        ProviderAsk::Local(url) => {
            let base_url = prompter.ask(&format!("Base URL [{url}]: "), |line| {
                text_field(line, Some(url))
            })?;
            (Some(base_url), None)
        }
        ProviderAsk::Cloud(suggested) => {
            let prompt =
                format!("API key environment variable name [{suggested}] (never the key itself): ");
            let api_key_env =
                prompter.ask(&prompt, |line| env_field(line, Some(suggested), true))?;
            (None, api_key_env)
        }
        ProviderAsk::Compatible => {
            let base_url = prompter.ask("Base URL: ", |line| text_field(line, None))?;
            let prompt =
                "API key environment variable name (blank to skip; never the key itself): ";
            let api_key_env = prompter.ask(prompt, |line| env_field(line, None, false))?;
            (Some(base_url), api_key_env)
        }
    };
    if let Some(env) = &api_key_env {
        let state = if std::env::var_os(env).is_some() {
            "is currently set"
        } else {
            "is not set"
        };
        prompter
            .say(&format!("{env} {state}."))
            .map_err(|_| Cancel)?;
    }
    Ok(Some(ProviderDraft {
        provider,
        model,
        base_url,
        api_key_env,
    }))
}
