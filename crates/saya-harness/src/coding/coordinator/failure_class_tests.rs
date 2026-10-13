//! Tests for the bounded failure-class mapping (`saya-harness::coding::coordinator::failure_class`).

use super::*;
use crate::coding::child::ChildFactoryError;
use saya_agent::{
    BudgetDimension, CancellationReason, EffectOwnerFailure, ProviderError, TokenUsage,
};
use std::io;

fn provider_failure() -> ScheduleChildError {
    ScheduleChildError::Run(ChildRunError::Agent(AgentError::Provider(
        ProviderError::Request("boom".into()),
    )))
}

fn output(truncated: bool) -> AgentOutput {
    AgentOutput {
        answer: String::new(),
        events: Vec::new(),
        used_bounded_sql_query: false,
        tool_metadata: Vec::new(),
        usage: TokenUsage::default(),
        learning_usage: None,
        truncated,
        answer_sql: None,
    }
}

#[test]
fn classifies_each_child_failure_into_its_closed_class() {
    let cases: Vec<(ScheduleChildError, CodingFailureClass)> = vec![
        (
            ScheduleChildError::Run(ChildRunError::Agent(AgentError::Provider(
                ProviderError::OutputTruncated {
                    partial_text: String::new(),
                    partial_tool_json: Vec::new(),
                },
            ))),
            CodingFailureClass::OutputLimit,
        ),
        (provider_failure(), CodingFailureClass::ProviderError),
        (
            ScheduleChildError::Run(ChildRunError::Agent(AgentError::Limit("cap"))),
            CodingFailureClass::LimitReached,
        ),
        (
            ScheduleChildError::Run(ChildRunError::Agent(AgentError::InvalidToolCall)),
            CodingFailureClass::InvalidToolCall,
        ),
        (
            ScheduleChildError::Run(ChildRunError::Agent(AgentError::InvalidHistory)),
            CodingFailureClass::InvalidToolCall,
        ),
        (
            ScheduleChildError::Run(ChildRunError::Agent(AgentError::EffectAdmissionDenied)),
            CodingFailureClass::AdmissionDenied,
        ),
        (
            ScheduleChildError::Run(ChildRunError::BudgetDenied),
            CodingFailureClass::AdmissionDenied,
        ),
        (
            ScheduleChildError::Run(ChildRunError::Agent(
                AgentError::UnsupportedPhysicalCoverage,
            )),
            CodingFailureClass::Unsupported,
        ),
        (
            ScheduleChildError::Run(ChildRunError::Factory(ChildFactoryError::new(
                io::Error::other("no components"),
            ))),
            CodingFailureClass::SetupFailed,
        ),
        (
            ScheduleChildError::Run(ChildRunError::ChannelIdentityMismatch),
            CodingFailureClass::SetupFailed,
        ),
        (
            ScheduleChildError::Run(ChildRunError::Cancelled {
                reason: CancellationReason::Budget(BudgetDimension::ElapsedTime),
            }),
            CodingFailureClass::LimitReached,
        ),
        (
            ScheduleChildError::Run(ChildRunError::Cancelled {
                reason: CancellationReason::OwnerFailure,
            }),
            CodingFailureClass::Cancelled,
        ),
        (
            ScheduleChildError::Run(ChildRunError::Agent(AgentError::Cancelled)),
            CodingFailureClass::Cancelled,
        ),
        (
            ScheduleChildError::Run(ChildRunError::OwnerFailure),
            CodingFailureClass::Internal,
        ),
        (ScheduleChildError::Incomplete, CodingFailureClass::Internal),
        (
            ScheduleChildError::Run(ChildRunError::Agent(AgentError::EffectOwnerFailure(
                EffectOwnerFailure::new(io::Error::other("owner")),
            ))),
            CodingFailureClass::Internal,
        ),
    ];
    for (error, expected) in cases {
        assert_eq!(classify(&error), Some(expected), "{error:?}");
    }
}

#[test]
fn result_level_mapping_follows_the_persisted_outcome() {
    // Truncated bounded-effort output is the Failed-by-truncation case.
    assert_eq!(
        classify_result(&Ok(output(true)), CodingResultOutcome::Failed),
        Some(CodingFailureClass::OutputLimit)
    );
    // A clean completion carries no failure class.
    assert_eq!(
        classify_result(&Ok(output(false)), CodingResultOutcome::Completed),
        None
    );
    // A user cancellation is a stated outcome, not a failure class.
    assert_eq!(
        classify_result(
            &Err(ScheduleChildError::Run(ChildRunError::Cancelled {
                reason: CancellationReason::User,
            })),
            CodingResultOutcome::Cancelled
        ),
        None
    );
    // Uncertain outcomes keep the class unset.
    assert_eq!(
        classify_result(
            &Err(ScheduleChildError::Incomplete),
            CodingResultOutcome::Uncertain
        ),
        None
    );
    assert_eq!(
        classify_result(&Err(provider_failure()), CodingResultOutcome::Failed),
        Some(CodingFailureClass::ProviderError)
    );
}
