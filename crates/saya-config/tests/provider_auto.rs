//! Choosing a provider from the environment when no layer stated one.
//!
//! One known provider key in the environment is a choice the operator already
//! made: the provider it belongs to, its default model, and the key's own
//! reference are filled in without a config file. Two or more keys are not a
//! choice — they are refused, naming both — and any provider a layer states
//! wins over all of it.

use saya_config::{
    AiProvider, ConfigFile, ConnectionsFile, ResolutionInput, SecretRef, SettingSource, resolve,
};

fn resolve_env(vars: &[(&str, &str)]) -> saya_config::ResolvedConfig {
    resolve(ResolutionInput::new(ConnectionsFile::default()).with_process_env(vars.iter().copied()))
        .expect("the config resolves")
}

/// The table of known keys, each paired with the provider and default model it
/// means. Asserted together so a key added to the table without its provider
/// cannot pass.
#[test]
fn each_known_key_chooses_its_provider_and_default_model() {
    for (var, provider, model) in [
        ("OPENAI_API_KEY", AiProvider::Openai, "gpt-4o-mini"),
        (
            "ANTHROPIC_API_KEY",
            AiProvider::Anthropic,
            "claude-sonnet-4-5",
        ),
        ("GEMINI_API_KEY", AiProvider::Gemini, "gemini-2.5-flash"),
        (
            "OPENROUTER_API_KEY",
            AiProvider::OpenaiCompatible,
            "openai/gpt-4o-mini",
        ),
        (
            "DEEPSEEK_API_KEY",
            AiProvider::OpenaiCompatible,
            "deepseek-chat",
        ),
    ] {
        let resolved = resolve_env(&[(var, "sk-test")]);
        assert_eq!(resolved.ai.provider, provider, "{var}");
        assert_eq!(resolved.ai.model, model, "{var}");
        assert_eq!(
            resolved.ai.api_key,
            Some(SecretRef::Env { env: var.into() }),
            "{var}"
        );
        assert_eq!(
            resolved.provenance.get("ai.provider").unwrap().source,
            SettingSource::AutoEnv { var: var.into() },
            "{var}"
        );
    }
}

/// The two keys whose provider needs an endpoint of its own carry one; the
/// native providers keep theirs absent so the provider's own default applies.
#[test]
fn a_compatible_key_carries_its_endpoint_and_a_native_key_does_not() {
    let openrouter = resolve_env(&[("OPENROUTER_API_KEY", "sk-test")]);
    assert_eq!(
        openrouter.ai.base_url.as_deref(),
        Some("https://openrouter.ai/api/v1")
    );

    let deepseek = resolve_env(&[("DEEPSEEK_API_KEY", "sk-test")]);
    assert_eq!(
        deepseek.ai.base_url.as_deref(),
        Some("https://api.deepseek.com/v1")
    );

    let openai = resolve_env(&[("OPENAI_API_KEY", "sk-test")]);
    assert_eq!(openai.ai.base_url, None);
}

/// Data sharing is the existing default, off, and auto-choice never moves it.
#[test]
fn data_sharing_stays_off_when_the_provider_is_chosen_from_a_key() {
    let resolved = resolve_env(&[("OPENAI_API_KEY", "sk-test")]);
    assert!(!resolved.ai.allow_data_sharing);
}

/// A configured provider always wins: the key is not a choice when a layer
/// already made one, whether the layer is a file, a flag, or `SAYA_PROVIDER`.
#[test]
fn a_configured_provider_wins_over_a_key_in_the_environment() {
    let file = ConfigFile::from_toml("[ai]\nprovider = 'anthropic'\n").unwrap();
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(file)
            .with_process_env([("OPENAI_API_KEY", "sk-openai")]),
    )
    .unwrap();
    assert_eq!(resolved.ai.provider, AiProvider::Anthropic);
    assert_eq!(resolved.ai.model, "qwen2.5-coder:14b");

    let resolved = resolve_env(&[
        ("SAYA_PROVIDER", "anthropic"),
        ("OPENAI_API_KEY", "sk-openai"),
    ]);
    assert_eq!(resolved.ai.provider, AiProvider::Anthropic);
    assert_eq!(resolved.ai.model, "qwen2.5-coder:14b");
}

/// Two keys are refused, not guessed between. The message names both keys and
/// the two ways to choose.
#[test]
fn two_provider_keys_are_refused_with_both_names() {
    let error = resolve(
        ResolutionInput::new(ConnectionsFile::default()).with_process_env([
            ("OPENAI_API_KEY", "sk-openai"),
            ("ANTHROPIC_API_KEY", "sk-anthropic"),
        ]),
    )
    .expect_err("two keys must not be guessed between");
    let message = error.to_string();
    assert!(message.contains("OPENAI_API_KEY"), "{message}");
    assert!(message.contains("ANTHROPIC_API_KEY"), "{message}");
    assert!(
        message.contains("--provider") || message.contains("SAYA_PROVIDER"),
        "{message}"
    );
}

/// An empty variable is not a key. A shell that exported one to nothing must
/// not turn the one real key into an ambiguity.
#[test]
fn an_empty_key_does_not_count_as_a_choice() {
    let resolved = resolve_env(&[
        ("OPENAI_API_KEY", ""),
        ("ANTHROPIC_API_KEY", "sk-anthropic"),
    ]);
    assert_eq!(resolved.ai.provider, AiProvider::Anthropic);
    assert_eq!(resolved.ai.model, "claude-sonnet-4-5");
}

/// No key at all changes nothing: the historical default remains.
#[test]
fn no_key_leaves_the_default_provider_in_place() {
    let resolved = resolve_env(&[]);
    assert_eq!(resolved.ai.provider, AiProvider::Ollama);
    assert_eq!(resolved.ai.model, "qwen2.5-coder:14b");
    assert_eq!(resolved.ai.api_key, None);
    assert!(!resolved.provider_chosen());
}

/// The first screen's one fact: a provider exists, from any layer or from the
/// environment's single key. No provider is the first-run state.
#[test]
fn provider_chosen_reports_whether_a_provider_exists() {
    assert!(resolve_env(&[("OPENAI_API_KEY", "sk-test")]).provider_chosen());

    let file = ConfigFile::from_toml("[ai]\nprovider = 'anthropic'\n").unwrap();
    let configured =
        resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(file)).unwrap();
    assert!(configured.provider_chosen());
}
