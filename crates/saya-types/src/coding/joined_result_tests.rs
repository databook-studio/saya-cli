//! Tests for the bounded joined-result envelope (`saya-types::coding::joined_result`).

use super::*;

const GOLDEN_WITHOUT_CLASS: &str = r#"{"version":1,"job_id":"job-1","agent_id":"agent-1","invocation_id":"inv-1","outcome":"failed","answer":"","answer_truncated":false}"#;
const GOLDEN_WITH_CLASS: &str = r#"{"version":1,"job_id":"job-1","agent_id":"agent-1","invocation_id":"inv-1","outcome":"failed","answer":"","answer_truncated":false,"failure_class":"provider_error"}"#;

fn sample() -> JoinedResultV1 {
    JoinedResultV1::new(
        JobId::parse("job-1").unwrap(),
        AgentId::parse("agent-1").unwrap(),
        InvocationId::parse("inv-1").unwrap(),
        CodingResultOutcome::Failed,
        String::new(),
        false,
    )
    .unwrap()
}

fn sample_json() -> String {
    String::from_utf8(sample().to_json_bytes().unwrap()).unwrap()
}

#[test]
fn failed_result_without_class_matches_the_legacy_wire_bytes() {
    assert_eq!(sample_json(), GOLDEN_WITHOUT_CLASS);
    assert_eq!(sample().failure_class(), None);
}

#[test]
fn failed_result_with_class_round_trips_and_carries_the_key() {
    let envelope = sample().with_failure_class(CodingFailureClass::ProviderError);
    let json = String::from_utf8(envelope.to_json_bytes().unwrap()).unwrap();
    assert_eq!(json, GOLDEN_WITH_CLASS);
    let reparsed = JoinedResultV1::from_json_bytes(json.as_bytes()).unwrap();
    assert_eq!(
        reparsed.failure_class(),
        Some(CodingFailureClass::ProviderError)
    );
    assert_eq!(reparsed, envelope);
}

#[test]
fn every_class_serializes_its_snake_case_name() {
    let cases = [
        (CodingFailureClass::OutputLimit, "output_limit"),
        (CodingFailureClass::ProviderError, "provider_error"),
        (CodingFailureClass::LimitReached, "limit_reached"),
        (CodingFailureClass::InvalidToolCall, "invalid_tool_call"),
        (CodingFailureClass::AdmissionDenied, "admission_denied"),
        (CodingFailureClass::Unsupported, "unsupported"),
        (CodingFailureClass::SetupFailed, "setup_failed"),
        (CodingFailureClass::Cancelled, "cancelled"),
        (CodingFailureClass::Internal, "internal"),
    ];
    for (class, name) in cases {
        let envelope = sample().with_failure_class(class);
        let json = String::from_utf8(envelope.to_json_bytes().unwrap()).unwrap();
        assert!(
            json.contains(&format!(r#""failure_class":"{name}""#)),
            "{class:?} must serialize as {name}: {json}"
        );
        assert_eq!(class.as_str(), name);
        let reparsed = JoinedResultV1::from_json_bytes(json.as_bytes()).unwrap();
        assert_eq!(reparsed.failure_class(), Some(class));
    }
}

#[test]
fn legacy_payload_without_the_class_key_parses_with_none() {
    let parsed = JoinedResultV1::from_json_bytes(GOLDEN_WITHOUT_CLASS.as_bytes()).unwrap();
    assert_eq!(parsed.failure_class(), None);
    assert_eq!(parsed.job_id().as_str(), "job-1");
    assert_eq!(parsed.outcome(), CodingResultOutcome::Failed);
    // Re-serializing a legacy payload stays byte-identical.
    assert_eq!(
        String::from_utf8(parsed.to_json_bytes().unwrap()).unwrap(),
        GOLDEN_WITHOUT_CLASS
    );
}

#[test]
fn unknown_failure_class_value_is_rejected() {
    let mutated = GOLDEN_WITHOUT_CLASS.replace(
        r#""answer_truncated":false}"#,
        r#""answer_truncated":false,"failure_class":"mystery"}"#,
    );
    assert_eq!(
        JoinedResultV1::from_json_bytes(mutated.as_bytes()),
        Err(JoinedResultError::Invalid)
    );
}

#[test]
fn nonfailed_result_with_a_failure_class_is_rejected() {
    let cases = ["completed", "cancelled", "uncertain"];
    let jsons = cases.map(|outcome| {
        let json = GOLDEN_WITH_CLASS.replace(
            r#""outcome":"failed""#,
            &format!(r#""outcome":"{outcome}""#),
        );
        assert!(json.contains(&format!(r#""outcome":"{outcome}""#)));
        assert!(json.contains(r#""failure_class":"provider_error""#));
        json
    });
    for (outcome, json) in cases.into_iter().zip(jsons) {
        assert_eq!(
            JoinedResultV1::from_json_bytes(json.as_bytes()),
            Err(JoinedResultError::Invalid),
            "{outcome} results cannot carry a failure class"
        );
    }
}

#[test]
fn completed_result_never_carries_a_class() {
    let envelope = JoinedResultV1::new(
        JobId::parse("job-1").unwrap(),
        AgentId::parse("agent-1").unwrap(),
        InvocationId::parse("inv-1").unwrap(),
        CodingResultOutcome::Completed,
        "done".to_owned(),
        false,
    )
    .unwrap();
    let json = String::from_utf8(envelope.to_json_bytes().unwrap()).unwrap();
    assert!(!json.contains("failure_class"));
}
