//! The joined-result wire contract: which outcomes may carry a closed `failure_class`.
//!
//! A `Failed` child has always been able to carry one. An `Uncertain` child — whose effect could
//! not be settled — may carry one too, so a reader can see *why* it is uncertain. `Completed`
//! and `Cancelled` results still refuse the key, and an older record without it still parses.
use saya_types::{
    AgentId, CodingFailureClass, CodingResultOutcome, InvocationId, JobId, JoinedResultError,
    JoinedResultV1,
};

const UNCERTAIN_WITHOUT_CLASS: &[u8] =
    br#"{"version":1,"job_id":"job-1","agent_id":"agent-1","invocation_id":"inv-1","outcome":"uncertain","answer":"","answer_truncated":false}"#;

fn uncertain_json(class: &str) -> Vec<u8> {
    format!(
        r#"{{"version":1,"job_id":"job-1","agent_id":"agent-1","invocation_id":"inv-1","outcome":"uncertain","answer":"","answer_truncated":false,"failure_class":"{class}"}}"#
    )
    .into_bytes()
}

#[test]
fn an_uncertain_record_may_carry_a_closed_class() {
    for class in [
        CodingFailureClass::ExtensionExchange,
        CodingFailureClass::MandatoryPersistence,
        CodingFailureClass::EffectsUncertain,
    ] {
        let bytes = uncertain_json(class.as_str());
        let decoded = JoinedResultV1::from_json_bytes(&bytes).unwrap();
        assert_eq!(decoded.outcome(), CodingResultOutcome::Uncertain);
        assert_eq!(decoded.failure_class(), Some(class));
        // Re-serializing the accepted record is byte-identical: the shape round-trips.
        assert_eq!(decoded.to_json_bytes().unwrap(), bytes);
    }
}

#[test]
fn a_completed_or_cancelled_record_still_refuses_the_key() {
    for outcome in ["completed", "cancelled"] {
        let bytes = String::from_utf8(uncertain_json("provider_error"))
            .unwrap()
            .replace(
                r#""outcome":"uncertain""#,
                &format!(r#""outcome":"{outcome}""#),
            )
            .into_bytes();
        assert_eq!(
            JoinedResultV1::from_json_bytes(&bytes),
            Err(JoinedResultError::Invalid),
            "{outcome} results cannot carry a failure class"
        );
    }
}

#[test]
fn an_uncertain_record_without_a_class_still_parses_unchanged() {
    let decoded = JoinedResultV1::from_json_bytes(UNCERTAIN_WITHOUT_CLASS).unwrap();
    assert_eq!(decoded.outcome(), CodingResultOutcome::Uncertain);
    assert_eq!(decoded.failure_class(), None);
    assert_eq!(
        decoded.to_json_bytes().unwrap(),
        UNCERTAIN_WITHOUT_CLASS.to_vec()
    );
    assert_eq!(decoded.job_id(), &JobId::parse("job-1").unwrap());
    assert_eq!(decoded.agent_id(), &AgentId::parse("agent-1").unwrap());
    assert_eq!(
        decoded.invocation_id(),
        &InvocationId::parse("inv-1").unwrap()
    );
}

#[test]
fn an_unknown_class_name_is_refused_for_an_uncertain_record_too() {
    assert_eq!(
        JoinedResultV1::from_json_bytes(&uncertain_json("mystery")),
        Err(JoinedResultError::Invalid)
    );
}
