//! Tests for the bounded joined-result envelope (`saya-types::coding::joined_result`).

use super::*;

const GOLDEN_WITHOUT_CLASS: &str = r#"{"version":1,"job_id":"job-1","agent_id":"agent-1","invocation_id":"inv-1","outcome":"failed","answer":"","answer_truncated":false}"#;
const GOLDEN_WITH_CLASS: &str = r#"{"version":1,"job_id":"job-1","agent_id":"agent-1","invocation_id":"inv-1","outcome":"failed","answer":"","answer_truncated":false,"failure_class":"provider_error"}"#;
/// An `Uncertain` record as written before a class could be attached to one.
const GOLDEN_UNCERTAIN_WITHOUT_CLASS: &str = r#"{"version":1,"job_id":"job-1","agent_id":"agent-1","invocation_id":"inv-1","outcome":"uncertain","answer":"","answer_truncated":false}"#;
const GOLDEN_UNCERTAIN_WITH_CLASS: &str = r#"{"version":1,"job_id":"job-1","agent_id":"agent-1","invocation_id":"inv-1","outcome":"uncertain","answer":"","answer_truncated":false,"failure_class":"extension_exchange"}"#;

fn envelope(outcome: CodingResultOutcome) -> JoinedResultV1 {
    JoinedResultV1::new(
        JobId::parse("job-1").unwrap(),
        AgentId::parse("agent-1").unwrap(),
        InvocationId::parse("inv-1").unwrap(),
        outcome,
        String::new(),
        false,
    )
    .unwrap()
}

fn sample() -> JoinedResultV1 {
    envelope(CodingResultOutcome::Failed)
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
        (
            CodingFailureClass::ProviderStreamEnded,
            "provider_stream_ended",
        ),
        (CodingFailureClass::LimitReached, "limit_reached"),
        (CodingFailureClass::InvalidToolCall, "invalid_tool_call"),
        (CodingFailureClass::AdmissionDenied, "admission_denied"),
        (CodingFailureClass::Unsupported, "unsupported"),
        (CodingFailureClass::SetupFailed, "setup_failed"),
        (CodingFailureClass::Cancelled, "cancelled"),
        (CodingFailureClass::ExtensionExchange, "extension_exchange"),
        (
            CodingFailureClass::CoordinatorStalled,
            "coordinator_stalled",
        ),
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
fn a_completed_or_cancelled_result_with_a_failure_class_is_still_refused() {
    for class in ["provider_error", "provider_stream_ended"] {
        for outcome in ["completed", "cancelled"] {
            let json = GOLDEN_WITH_CLASS
                .replace(
                    r#""outcome":"failed""#,
                    &format!(r#""outcome":"{outcome}""#),
                )
                .replace("provider_error", class);
            assert!(json.contains(&format!(r#""outcome":"{outcome}""#)));
            assert!(json.contains(&format!(r#""failure_class":"{class}""#)));
            assert_eq!(
                JoinedResultV1::from_json_bytes(json.as_bytes()),
                Err(JoinedResultError::Invalid),
                "{outcome} results cannot carry the {class} failure class"
            );
        }
    }
}

/// A child whose effect could not be settled may carry the class of its cause, exactly as a
/// failed child does: the outcome and the class are independent fields on the wire.
#[test]
fn an_uncertain_result_round_trips_with_its_failure_class() {
    let envelope = envelope(CodingResultOutcome::Uncertain)
        .with_failure_class(CodingFailureClass::ExtensionExchange);
    let json = String::from_utf8(envelope.to_json_bytes().unwrap()).unwrap();
    assert_eq!(json, GOLDEN_UNCERTAIN_WITH_CLASS);
    let reparsed = JoinedResultV1::from_json_bytes(json.as_bytes()).unwrap();
    assert_eq!(reparsed.outcome(), CodingResultOutcome::Uncertain);
    assert_eq!(
        reparsed.failure_class(),
        Some(CodingFailureClass::ExtensionExchange)
    );
    assert_eq!(reparsed, envelope);
}

/// The `Failed` wire shape did not move: byte-for-byte the same goldens as before the change.
#[test]
fn a_failed_result_with_a_class_is_unchanged() {
    assert_eq!(sample_json(), GOLDEN_WITHOUT_CLASS);
    assert_eq!(
        String::from_utf8(
            sample()
                .with_failure_class(CodingFailureClass::ProviderError)
                .to_json_bytes()
                .unwrap()
        )
        .unwrap(),
        GOLDEN_WITH_CLASS
    );
}

/// A record an older binary wrote has no class key at all and still parses, unchanged.
#[test]
fn an_uncertain_result_without_a_class_still_parses() {
    let parsed =
        JoinedResultV1::from_json_bytes(GOLDEN_UNCERTAIN_WITHOUT_CLASS.as_bytes()).unwrap();
    assert_eq!(parsed.outcome(), CodingResultOutcome::Uncertain);
    assert_eq!(parsed.failure_class(), None);
    assert_eq!(
        String::from_utf8(parsed.to_json_bytes().unwrap()).unwrap(),
        GOLDEN_UNCERTAIN_WITHOUT_CLASS
    );
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
