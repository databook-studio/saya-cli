//! The operator's one-line advice after a native coding failure names the cause of that class.

use super::CodingError;
use saya_types::{AgentId, CodingFailureClass, CodingRunFailure, InvocationId};

fn failed(class: CodingFailureClass) -> String {
    CodingError::Failed(Box::new(CodingRunFailure::attributed(
        class,
        AgentId::parse("impl").unwrap(),
        InvocationId::parse("inv-1").unwrap(),
    )))
    .to_string()
}

#[test]
fn a_stream_that_ended_early_points_at_the_time_limit_and_the_output_cap_not_the_model() {
    let message = failed(CodingFailureClass::ProviderStreamEnded);
    assert!(message.contains("provider_stream_ended"), "{message}");
    assert!(message.contains("time limit"), "{message}");
    assert!(message.contains("ai.max_output_tokens"), "{message}");
    assert!(!message.contains("configured model"), "{message}");
    assert!(!message.contains("setup --probe"), "{message}");
}

#[test]
fn a_generic_provider_error_keeps_its_model_probe_advice() {
    let message = failed(CodingFailureClass::ProviderError);
    assert!(
        message.contains("check the configured model with `saya job setup --probe`"),
        "{message}"
    );
}
