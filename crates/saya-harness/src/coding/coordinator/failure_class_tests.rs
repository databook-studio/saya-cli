//! Tests for the bounded failure-class mapping (`saya-harness::coding::coordinator::failure_class`).

use super::*;
use crate::coding::child::{ChildFactoryError, OwnerFailureSnapshot};
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
            ScheduleChildError::Run(ChildRunError::Agent(AgentError::Provider(
                ProviderError::StreamEnded,
            ))),
            CodingFailureClass::ProviderStreamEnded,
        ),
        (
            ScheduleChildError::Run(ChildRunError::Agent(AgentError::Provider(
                ProviderError::InvalidResponse,
            ))),
            CodingFailureClass::ProviderError,
        ),
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
            CodingFailureClass::BudgetStopped,
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
            CodingFailureClass::MandatoryPersistence,
        ),
        (
            ScheduleChildError::Incomplete,
            CodingFailureClass::OutputLimit,
        ),
        (
            ScheduleChildError::Run(ChildRunError::Agent(AgentError::Provider(
                ProviderError::Cancelled,
            ))),
            CodingFailureClass::Cancelled,
        ),
        (
            ScheduleChildError::Run(ChildRunError::Agent(AgentError::EffectOwnerFailure(
                EffectOwnerFailure::new(io::Error::other("owner")),
            ))),
            CodingFailureClass::MandatoryPersistence,
        ),
    ];
    for (error, expected) in cases {
        assert_eq!(classify(&error), Some(expected), "{error:?}");
    }
}

/// A stream cut that exhausts its continuations is the same failure to an
/// operator as a stream that ended before its terminator: the provider stopped
/// mid-answer. It keeps the same class — no wire change — so the guidance and
/// the recorded category stay stable.
#[test]
fn an_exhausted_stream_cut_classifies_as_provider_stream_ended() {
    let cut = ScheduleChildError::Run(ChildRunError::Agent(AgentError::Provider(
        ProviderError::StreamCut {
            elapsed_secs: 302,
            estimated_tokens: 62_000,
        },
    )));
    assert_eq!(
        classify(&cut),
        Some(CodingFailureClass::ProviderStreamEnded)
    );
    assert_eq!(
        classify_result(&Err(cut), CodingResultOutcome::Uncertain),
        Some(CodingFailureClass::ProviderStreamEnded)
    );
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
    // An uncertain outcome carries the class of its cause, exactly as a failed one does.
    assert_eq!(
        classify_result(
            &Err(ScheduleChildError::Run(ChildRunError::Unsettled)),
            CodingResultOutcome::Uncertain
        ),
        Some(CodingFailureClass::EffectsUncertain)
    );
    assert_eq!(
        classify_result(&Err(provider_failure()), CodingResultOutcome::Failed),
        Some(CodingFailureClass::ProviderError)
    );
}

/// A child that ends uncertain carries the class of its latched cause — the same class the
/// job-level failure names. The two production cases: a refused post-edit extension exchange and
/// a failed known-commit receipt.
#[test]
fn an_uncertain_outcome_carries_the_class_of_its_cause() {
    let extension = ScheduleChildError::Run(ChildRunError::OwnerOperation {
        prior: None,
        failure: OwnerFailureSnapshot::new(CodingFailureClass::ExtensionExchange, None),
    });
    assert_eq!(
        classify_result(&Err(extension), CodingResultOutcome::Uncertain),
        Some(CodingFailureClass::ExtensionExchange)
    );
    let receipt = ScheduleChildError::Run(ChildRunError::OwnerOperation {
        prior: None,
        failure: OwnerFailureSnapshot::new(CodingFailureClass::MandatoryPersistence, None),
    });
    assert_eq!(
        classify_result(&Err(receipt), CodingResultOutcome::Uncertain),
        Some(CodingFailureClass::MandatoryPersistence)
    );
    // An owner-failure observed by the child's own loop, unsettled by construction.
    let effect_owner = ScheduleChildError::Run(ChildRunError::Agent(
        AgentError::EffectOwnerFailure(EffectOwnerFailure::new(io::Error::other("owner"))),
    ));
    assert_eq!(
        classify_result(&Err(effect_owner), CodingResultOutcome::Uncertain),
        Some(CodingFailureClass::MandatoryPersistence)
    );
    // A prior provider cause outranks the owner snapshot, as it does at the job level.
    let prior = ScheduleChildError::Run(ChildRunError::OwnerOperation {
        prior: Some(Box::new(ChildRunError::Agent(AgentError::Provider(
            ProviderError::InvalidResponse,
        )))),
        failure: OwnerFailureSnapshot::new(CodingFailureClass::ExtensionExchange, None),
    });
    assert_eq!(
        classify_result(&Err(prior), CodingResultOutcome::Uncertain),
        Some(CodingFailureClass::ProviderError)
    );
    // There is no cause to name when the child produced a clean output, whatever the outcome says.
    assert_eq!(
        classify_result(&Ok(output(false)), CodingResultOutcome::Uncertain),
        None
    );
}

#[test]
fn native_owner_snapshot_is_conservative_and_never_erases_a_provider_cause() {
    let owner = OwnerFailureSnapshot::new(
        CodingFailureClass::ExtensionExchange,
        Some(CodingFailureClass::MandatoryPersistence),
    );
    let only_owner = ScheduleChildError::Run(ChildRunError::OwnerOperation {
        prior: None,
        failure: owner,
    });
    assert_eq!(
        classify(&only_owner),
        Some(CodingFailureClass::ExtensionExchange)
    );
    assert_eq!(
        super::super::results::envelope(
            saya_types::JobId::parse("job-a").unwrap(),
            saya_types::AgentId::parse("agent-a").unwrap(),
            saya_types::InvocationId::parse("invocation-a").unwrap(),
            &Err(only_owner),
        )
        .unwrap()
        .outcome(),
        CodingResultOutcome::Uncertain
    );
    let provider = ScheduleChildError::Run(ChildRunError::OwnerOperation {
        prior: Some(Box::new(ChildRunError::Agent(AgentError::Provider(
            ProviderError::InvalidResponse,
        )))),
        failure: owner,
    });
    assert_eq!(classify(&provider), Some(CodingFailureClass::ProviderError));

    let owner_induced = ScheduleChildError::Run(ChildRunError::OwnerOperation {
        prior: None,
        failure: owner,
    });
    assert_eq!(
        classify(&owner_induced),
        Some(CodingFailureClass::ExtensionExchange)
    );
    let user = ScheduleChildError::Run(ChildRunError::OwnerOperation {
        prior: Some(Box::new(ChildRunError::Cancelled {
            reason: CancellationReason::User,
        })),
        failure: owner,
    });
    assert_eq!(classify(&user), Some(CodingFailureClass::Cancelled));
    let budget = ScheduleChildError::Run(ChildRunError::OwnerOperation {
        prior: Some(Box::new(ChildRunError::Cancelled {
            reason: CancellationReason::Budget(BudgetDimension::ElapsedTime),
        })),
        failure: owner,
    });
    assert_eq!(classify(&budget), Some(CodingFailureClass::BudgetStopped));
}
