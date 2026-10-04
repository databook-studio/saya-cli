use super::super::turn_config::AgentRuntimeError;
use super::request_preflight_config;
use std::num::NonZeroU64;

use saya_agent::RequestPreflightConfig;
use saya_config::{AiProvider, CompactionMode, ResolvedAi};

fn ai(model: &str, explicit_window: Option<u64>, max_output_tokens: u32) -> ResolvedAi {
    ResolvedAi {
        provider: AiProvider::Ollama,
        model: model.into(),
        base_url: None,
        api_key: None,
        allow_data_sharing: true,
        temperature: 0.0,
        timeout_seconds: 60,
        idle_timeout_seconds: 90,
        max_output_tokens,
        max_output_tokens_is_default: false,
        context_byte_budget: 200_000,
        context_window_tokens: explicit_window,
        show_thinking: false,
        compaction: CompactionMode::Auto,
        retry_delays_ms: vec![250, 500, 1000],
    }
}

fn reserve(tokens: u64) -> NonZeroU64 {
    NonZeroU64::new(tokens).unwrap()
}

#[test]
fn explicit_window_wins_over_the_table() {
    let config = request_preflight_config(&ai("totally-made-up-model", Some(8000), 4096)).unwrap();
    assert_eq!(
        config,
        RequestPreflightConfig::new(Some(reserve(8000)), reserve(4096))
    );
}

#[test]
fn table_window_for_a_known_model_when_nothing_is_declared() {
    let config = request_preflight_config(&ai("gpt-4o", None, 4096)).unwrap();
    assert_eq!(
        config,
        RequestPreflightConfig::new(Some(reserve(128_000)), reserve(4096))
    );
}

#[test]
fn unknown_model_stays_unknown() {
    let config = request_preflight_config(&ai("totally-made-up-model", None, 4096)).unwrap();
    assert_eq!(config, RequestPreflightConfig::new(None, reserve(4096)));
}

#[test]
fn output_reserve_is_the_effective_max_output_tokens() {
    let config = request_preflight_config(&ai("gpt-4o", None, 1234)).unwrap();
    assert_eq!(
        config,
        RequestPreflightConfig::new(Some(reserve(128_000)), reserve(1234))
    );
}

#[test]
fn zero_output_reserve_is_a_configuration_error() {
    let result = request_preflight_config(&ai("gpt-4o", None, 0));
    assert!(matches!(result, Err(AgentRuntimeError::Configuration(_))));
}

#[test]
fn zero_declared_window_is_a_configuration_error() {
    let result = request_preflight_config(&ai("gpt-4o", Some(0), 4096));
    assert!(matches!(result, Err(AgentRuntimeError::Configuration(_))));
}
