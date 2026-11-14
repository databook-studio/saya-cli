use super::super::turn_config::AgentRuntimeError;
use super::request_preflight_config;
use std::num::NonZeroU64;

use saya_agent::RequestPreflightConfig;
use saya_config::{AiProvider, CompactionMode, ResolvedAi};

fn ai(model: &str, explicit_window: Option<u64>, max_output_tokens: Option<u32>) -> ResolvedAi {
    ResolvedAi {
        app_attribution: true,
        provider: AiProvider::Ollama,
        model: model.into(),
        base_url: None,
        api_key: None,
        allow_data_sharing: true,
        temperature: 0.0,
        timeout_seconds: 60,
        idle_timeout_seconds: 90,
        request_timeout_seconds: 180,
        max_output_tokens,
        reasoning_effort: None,
        context_byte_budget: 200_000,
        context_window_tokens: explicit_window,
        show_thinking: false,
        compaction: CompactionMode::Auto,
        compaction_trigger_percent: 50,
        retry_delays_ms: vec![250, 500, 1000],
    }
}

fn reserve(tokens: u64) -> NonZeroU64 {
    NonZeroU64::new(tokens).unwrap()
}

#[test]
fn explicit_window_wins_over_the_table() {
    let config =
        request_preflight_config(&ai("totally-made-up-model", Some(8000), Some(4096))).unwrap();
    assert_eq!(
        config,
        RequestPreflightConfig::new(Some(reserve(8000)), Some(reserve(4096)))
    );
}

#[test]
fn table_window_for_a_known_model_when_nothing_is_declared() {
    let config = request_preflight_config(&ai("gpt-4o", None, Some(4096))).unwrap();
    assert_eq!(
        config,
        RequestPreflightConfig::new(Some(reserve(128_000)), Some(reserve(4096)))
    );
}

#[test]
fn an_unknown_model_uses_the_configured_window_for_compaction() {
    let mut settings = ai("totally-made-up-model", Some(200_000), Some(4096));
    settings.compaction_trigger_percent = 25;
    let config = request_preflight_config(&settings).unwrap();
    assert_eq!(
        config.compaction_trigger_tokens(),
        Some(50_000),
        "an unknown model compacts at the configured window's fraction"
    );
}

#[test]
fn a_known_model_compacts_at_the_default_fraction_of_its_table_window() {
    let config = request_preflight_config(&ai("gpt-4o", None, Some(4096))).unwrap();
    assert_eq!(
        config.compaction_trigger_tokens(),
        Some(64_000),
        "a known model uses its table window when nothing is declared"
    );
}

#[test]
fn unknown_model_stays_unknown() {
    let config = request_preflight_config(&ai("totally-made-up-model", None, Some(4096))).unwrap();
    assert_eq!(
        config,
        RequestPreflightConfig::new(None, Some(reserve(4096)))
    );
}

/// With no configured output ceiling there is nothing to reserve, so the
/// whole window above the prompt margin is available to the prompt.
#[test]
fn an_unset_output_ceiling_reserves_nothing() {
    let config = request_preflight_config(&ai("gpt-4o", None, None)).unwrap();
    assert_eq!(
        config,
        RequestPreflightConfig::new(Some(reserve(128_000)), None)
    );
}

#[test]
fn output_reserve_is_the_effective_max_output_tokens() {
    let config = request_preflight_config(&ai("gpt-4o", None, Some(1234))).unwrap();
    assert_eq!(
        config,
        RequestPreflightConfig::new(Some(reserve(128_000)), Some(reserve(1234)))
    );
}

#[test]
fn zero_output_reserve_is_a_configuration_error() {
    let result = request_preflight_config(&ai("gpt-4o", None, Some(0)));
    assert!(matches!(result, Err(AgentRuntimeError::Configuration(_))));
}

#[test]
fn zero_declared_window_is_a_configuration_error() {
    let result = request_preflight_config(&ai("gpt-4o", Some(0), Some(4096)));
    assert!(matches!(result, Err(AgentRuntimeError::Configuration(_))));
}

#[test]
fn an_output_ceiling_at_or_above_the_model_window_is_a_configuration_error() {
    let result = request_preflight_config(&ai("gpt-4o", Some(8192), Some(8192)));
    let AgentRuntimeError::Configuration(message) = result.expect_err("must be refused") else {
        panic!("expected a configuration error");
    };
    assert!(message.contains("max_output_tokens"), "got: {message}");
    assert!(message.contains("context window"), "got: {message}");
}

#[test]
fn an_output_ceiling_below_the_window_composes() {
    let config = request_preflight_config(&ai("gpt-4o", Some(8192), Some(2048))).unwrap();
    assert_eq!(
        config,
        RequestPreflightConfig::new(Some(reserve(8192)), Some(reserve(2048)))
    );
}

#[test]
fn an_output_ceiling_one_margin_below_the_window_is_refused() {
    // 7168 + 1024 (the prompt margin) >= 8192, so no room for a prompt.
    let result = request_preflight_config(&ai("gpt-4o", Some(8192), Some(7168)));
    assert!(matches!(result, Err(AgentRuntimeError::Configuration(_))));
}
