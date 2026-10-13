//! The joined result a finished child persists: its outcome and, when it failed, why.

use super::*;
use saya_agent::{AgentError, ProviderError, TokenUsage};

fn envelope_of(result: Result<AgentOutput, ScheduleChildError>) -> JoinedResultV1 {
    envelope(
        JobId::parse("job-1").unwrap(),
        AgentId::parse("agent-1").unwrap(),
        InvocationId::parse("inv-1").unwrap(),
        &result,
    )
    .unwrap()
}

fn output(truncated: bool) -> AgentOutput {
    AgentOutput {
        answer: "done".into(),
        events: Vec::new(),
        used_bounded_sql_query: false,
        tool_metadata: Vec::new(),
        usage: TokenUsage::default(),
        learning_usage: None,
        truncated,
        answer_sql: None,
    }
}

fn failed_with(error: AgentError) -> ScheduleChildError {
    ScheduleChildError::Run(ChildRunError::Agent(error))
}

#[test]
fn a_failed_child_persists_its_failure_class_and_no_answer() {
    let result = envelope_of(Err(failed_with(AgentError::Provider(
        ProviderError::Request("private detail".into()),
    ))));
    assert_eq!(result.outcome(), CodingResultOutcome::Failed);
    assert_eq!(
        result.failure_class(),
        Some(CodingFailureClass::ProviderError)
    );
    assert_eq!(result.answer(), "");
    let json = String::from_utf8(result.to_json_bytes().unwrap()).unwrap();
    assert!(
        !json.contains("private detail"),
        "no provider text is persisted: {json}"
    );
}

#[test]
fn a_truncated_output_fails_with_the_output_limit_class() {
    let result = envelope_of(Ok(output(true)));
    assert_eq!(result.outcome(), CodingResultOutcome::Failed);
    assert_eq!(
        result.failure_class(),
        Some(CodingFailureClass::OutputLimit)
    );
}

#[test]
fn completed_uncertain_and_user_cancelled_children_carry_no_class() {
    let completed = envelope_of(Ok(output(false)));
    assert_eq!(completed.outcome(), CodingResultOutcome::Completed);
    assert_eq!(completed.failure_class(), None);

    let uncertain = envelope_of(Err(ScheduleChildError::Run(ChildRunError::Unsettled)));
    assert_eq!(uncertain.outcome(), CodingResultOutcome::Uncertain);
    assert_eq!(uncertain.failure_class(), None);

    let cancelled = envelope_of(Err(ScheduleChildError::Run(ChildRunError::Cancelled {
        reason: CancellationReason::User,
    })));
    assert_eq!(cancelled.outcome(), CodingResultOutcome::Cancelled);
    assert_eq!(cancelled.failure_class(), None);
}
