//! What a child failed by a provider stream cut persists: its own class, never provider text.

use super::*;
use crate::coding::coordinator::results::envelope;
use saya_types::{AgentId, InvocationId, JobId, JoinedResultV1};

fn persisted(error: ProviderError) -> JoinedResultV1 {
    envelope(
        JobId::parse("job-a").unwrap(),
        AgentId::parse("agent-a").unwrap(),
        InvocationId::parse("invocation-a").unwrap(),
        &Err(ScheduleChildError::Run(ChildRunError::Agent(
            AgentError::Provider(error),
        ))),
    )
    .unwrap()
}

fn json(result: &JoinedResultV1) -> String {
    String::from_utf8(result.to_json_bytes().unwrap()).unwrap()
}

#[test]
fn a_child_whose_stream_ended_persists_a_failed_result_with_its_own_class_and_no_answer() {
    let result = persisted(ProviderError::StreamEnded);
    assert_eq!(result.outcome(), CodingResultOutcome::Failed);
    assert_eq!(result.answer(), "");
    assert_eq!(
        json(&result),
        r#"{"version":1,"job_id":"job-a","agent_id":"agent-a","invocation_id":"invocation-a","outcome":"failed","answer":"","answer_truncated":false,"failure_class":"provider_stream_ended"}"#
    );
}

#[test]
fn provider_text_reaches_no_persisted_result_and_keeps_the_generic_class() {
    for error in [
        ProviderError::Request("PRIVATE_PROVIDER_SENTINEL".into()),
        ProviderError::Configuration("PRIVATE_PROVIDER_SENTINEL".into()),
    ] {
        let result = persisted(error);
        assert_eq!(
            result.failure_class(),
            Some(CodingFailureClass::ProviderError)
        );
        assert!(
            !json(&result).contains("PRIVATE_PROVIDER_SENTINEL"),
            "{}",
            json(&result)
        );
    }
}
