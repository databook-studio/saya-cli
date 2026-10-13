use super::{CodingError, map_root_error};
use saya_agent::{AgentError, ProviderError};

#[test]
fn a_whole_limit_truncation_maps_to_the_output_limit_error_naming_the_configured_number() {
    let error = AgentError::Provider(ProviderError::output_truncated(
        "a partial answer that never completes".into(),
        Vec::new(),
    ));
    let mapped = map_root_error(error, 4096);
    assert!(
        matches!(mapped, CodingError::OutputLimit { configured: 4096 }),
        "truncation must map to the typed output-limit error carrying the configured number: {mapped}"
    );
    assert!(
        mapped.to_string().contains("ai.max_output_tokens = 4096"),
        "the message names only the configured limit and its config key: {mapped}"
    );
    let mapped = map_root_error(
        AgentError::Provider(ProviderError::output_truncated(String::new(), Vec::new())),
        2048,
    );
    assert!(
        matches!(mapped, CodingError::OutputLimit { configured: 2048 }),
        "the configured limit travels with the error: {mapped}"
    );
}

#[test]
fn every_other_agent_error_still_maps_to_the_generic_run_failure_without_provider_detail() {
    let cases = vec![
        AgentError::Provider(ProviderError::Request("transport down".into())),
        AgentError::Provider(ProviderError::InvalidResponse),
        AgentError::Provider(ProviderError::Configuration("no endpoint".into())),
        AgentError::Limit("answer bound reached"),
        AgentError::InvalidToolCall,
        AgentError::Cancelled,
    ];
    for error in cases {
        let mapped = map_root_error(error, 4096);
        assert!(
            matches!(mapped, CodingError::RunFailed),
            "only truncation names the limit; everything else stays generic: {mapped}"
        );
    }
}
