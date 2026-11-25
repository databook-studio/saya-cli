//! Choosing a provider from exactly one known provider key in the environment.
//!
//! This is the first-run path: with no provider stated by any layer, a single
//! key is a choice the operator already made. The provider it belongs to, its
//! default model, its endpoint where one is needed, and the key's own
//! environment reference are filled in and attributed to that key. Two or more
//! keys are not a choice — they are refused, naming every key — and a provider
//! any layer stated wins over all of it.

use std::collections::BTreeMap;

use crate::{
    AiProvider, ConfigError, ConfigFile, SecretRef,
    provenance::{Recorder, SettingSource},
};

/// One provider key Saya knows, and the provider it means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderKey {
    /// The environment variable's name.
    pub env: &'static str,
    /// The provider the key authenticates.
    pub provider: AiProvider,
    /// The provider's default model, used when no layer stated one.
    pub model: &'static str,
    /// The endpoint the provider needs, when its own default is not the one
    /// the key belongs to. `None` keeps the provider's built-in default.
    pub base_url: Option<&'static str>,
}

/// The known provider keys. Order is presentation order: the prompt lists
/// them as written here, and two or more present keys are named in this order.
pub const KEYS: &[ProviderKey] = &[
    ProviderKey {
        env: "OPENAI_API_KEY",
        provider: AiProvider::Openai,
        model: "gpt-4o-mini",
        base_url: None,
    },
    ProviderKey {
        env: "ANTHROPIC_API_KEY",
        provider: AiProvider::Anthropic,
        model: "claude-sonnet-4-5",
        base_url: None,
    },
    ProviderKey {
        env: "GEMINI_API_KEY",
        provider: AiProvider::Gemini,
        model: "gemini-2.5-flash",
        base_url: None,
    },
    ProviderKey {
        env: "OPENROUTER_API_KEY",
        provider: AiProvider::OpenaiCompatible,
        model: "openai/gpt-4o-mini",
        base_url: Some("https://openrouter.ai/api/v1"),
    },
    ProviderKey {
        env: "DEEPSEEK_API_KEY",
        provider: AiProvider::OpenaiCompatible,
        model: "deepseek-chat",
        base_url: Some("https://api.deepseek.com/v1"),
    },
];

/// The known keys present with a non-empty value, in table order. An exported
/// but empty variable is not a key: a shell that set one to nothing must not
/// turn the one real key into an ambiguity.
pub fn provider_keys_in_env(env: &BTreeMap<String, String>) -> Vec<&'static ProviderKey> {
    KEYS.iter()
        .filter(|key| {
            env.get(key.env)
                .is_some_and(|value| !value.trim().is_empty())
        })
        .collect()
}

/// The table entry for one environment variable, if it is a known key.
pub fn key_for_env(env: &str) -> Option<&'static ProviderKey> {
    KEYS.iter().find(|key| key.env == env)
}

/// Applies the auto-choice (or the prompt's answer) to the merged file before
/// the resolved settings are built.
///
/// `prompted` is the key the operator chose at the terminal; it is already a
/// decision, so it is not re-detected and its provider is attributed to the
/// prompt. Without it, the choice is made only when no layer stated a
/// provider: zero keys change nothing, one is used, and two or more are a
/// typed refusal.
pub(crate) fn apply(
    file: &mut ConfigFile,
    env: &BTreeMap<String, String>,
    recorder: &mut Recorder,
    prompted: Option<&'static ProviderKey>,
) -> Result<(), ConfigError> {
    let key = match prompted {
        Some(key) => key,
        None if file.ai.provider.is_some() => return Ok(()),
        None => match provider_keys_in_env(env).as_slice() {
            [] => return Ok(()),
            [key] => *key,
            many => {
                let names = many
                    .iter()
                    .map(|key| key.env)
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(ConfigError::AmbiguousProviderKeys { keys: names });
            }
        },
    };
    let from_key = || SettingSource::AutoEnv {
        var: key.env.to_owned(),
    };
    file.ai.provider = Some(key.provider);
    recorder.record(
        "ai.provider",
        match prompted {
            Some(_) => SettingSource::Prompt,
            None => from_key(),
        },
    );
    // A model, endpoint, or key the operator stated is theirs; only the unset
    // fields are filled from the key's entry.
    if file.ai.model.is_none() {
        file.ai.model = Some(key.model.to_owned());
        recorder.record("ai.model", from_key());
    }
    if file.ai.base_url.is_none()
        && let Some(base_url) = key.base_url
    {
        file.ai.base_url = Some(base_url.to_owned());
        recorder.record("ai.base_url", from_key());
    }
    if file.ai.api_key.is_none() {
        let reference = SecretRef::Env {
            env: key.env.to_owned(),
        };
        recorder.record_secret("ai.api_key", from_key(), &reference);
        file.ai.api_key = Some(reference);
    }
    Ok(())
}
