//! R04a: incomplete reported usage surfaces as snapshot estimates, never as
//! measured totals, and estimates never change admission thresholds.

use std::time::Duration;

use saya_agent::{
    EffectAdmission, EffectAdmissionError, EffectCallKind, EffectOutcome, EffectPermit,
    EffectSettlement, PhysicalAttempt, TokenUsage,
};
use saya_harness::coding::budget::{
    ChildBudgetView, ChildCountLimits, JobBudgetLedger, UsageCoverage,
};

use super::fixture::{Fixture, limits};

fn partial_settlement(attempt: PhysicalAttempt) -> EffectSettlement {
    EffectSettlement::PhysicalAttempt {
        attempt,
        outcome: EffectOutcome::Succeeded,
        reported_usage: Some(TokenUsage::new(10, 20)),
        usage_complete: false,
    }
}

fn complete_settlement(attempt: PhysicalAttempt) -> EffectSettlement {
    EffectSettlement::PhysicalAttempt {
        attempt,
        outcome: EffectOutcome::Succeeded,
        reported_usage: Some(TokenUsage::new(3, 4)),
        usage_complete: true,
    }
}

fn unknown_settlement(attempt: PhysicalAttempt) -> EffectSettlement {
    EffectSettlement::PhysicalAttempt {
        attempt,
        outcome: EffectOutcome::Unknown,
        reported_usage: None,
        usage_complete: false,
    }
}

/// One child view over generous count limits with the transport cap under
/// test supplied by the caller.
async fn child_view(
    fixture: &Fixture,
    agent: &str,
    invocation: &str,
    transport_attempts: u64,
) -> (JobBudgetLedger, ChildBudgetView) {
    let authority = fixture.add_child(agent, invocation).await;
    let ledger = fixture
        .ledger(limits(
            4,
            transport_attempts,
            4,
            4,
            1,
            Duration::from_secs(30),
        ))
        .await
        .unwrap();
    let session = ledger
        .begin_child(
            authority.agent_id(),
            authority.invocation_id(),
            ChildCountLimits {
                logical_requests: 4,
                tool_calls: 1,
            },
        )
        .await
        .unwrap();
    let child = session.view().clone();
    (ledger, child)
}

/// Admits one logical model request and one physical send over it.
async fn admitted_attempt(child: &ChildBudgetView) -> (EffectPermit, PhysicalAttempt) {
    let request = child.admit_model_request().await.unwrap();
    let attempt = PhysicalAttempt::new(request, EffectCallKind::Answer, 0);
    let permit = child.admit_physical_attempt(attempt).await.unwrap();
    (permit, attempt)
}

#[tokio::test]
async fn partial_reported_usage_surfaces_estimated_not_measured() {
    let fixture = Fixture::new("r04-estimate-partial").await;
    let (ledger, child) =
        child_view(&fixture, "agent-r04-partial", "invocation-r04-partial", 4).await;

    let (first_permit, first_attempt) = admitted_attempt(&child).await;
    child
        .settle(first_permit, partial_settlement(first_attempt))
        .await
        .unwrap();

    let snapshot = ledger.snapshot().await;
    assert_eq!(snapshot.estimated_input_tokens, Some(10));
    assert_eq!(snapshot.estimated_output_tokens, Some(20));
    assert_eq!(snapshot.reported_input_tokens, None);
    assert_eq!(snapshot.reported_output_tokens, None);
    assert_eq!(snapshot.usage_coverage, UsageCoverage::Partial);
    assert_eq!(snapshot.transport_attempts, 1);

    let (second_permit, second_attempt) = admitted_attempt(&child).await;
    child
        .settle(second_permit, complete_settlement(second_attempt))
        .await
        .unwrap();

    let snapshot = ledger.snapshot().await;
    assert_eq!(snapshot.reported_input_tokens, Some(3));
    assert_eq!(snapshot.reported_output_tokens, Some(4));
    assert_eq!(snapshot.estimated_input_tokens, Some(10));
    assert_eq!(snapshot.estimated_output_tokens, Some(20));

    fixture.close().await;
}

#[tokio::test]
async fn unknown_outcome_never_becomes_zero_or_estimate() {
    let fixture = Fixture::new("r04-estimate-unknown").await;
    let (ledger, child) =
        child_view(&fixture, "agent-r04-unknown", "invocation-r04-unknown", 4).await;

    let (permit, attempt) = admitted_attempt(&child).await;
    child
        .settle(permit, unknown_settlement(attempt))
        .await
        .unwrap();

    let snapshot = ledger.snapshot().await;
    assert_eq!(snapshot.estimated_input_tokens, None);
    assert_eq!(snapshot.estimated_output_tokens, None);
    assert_eq!(snapshot.reported_input_tokens, None);
    assert_eq!(snapshot.reported_output_tokens, None);
    assert_eq!(snapshot.unknown_transport_attempts, 1);
    assert_eq!(snapshot.usage_coverage, UsageCoverage::Unknown);
    assert_eq!(snapshot.transport_attempts, 1);

    fixture.close().await;
}

#[tokio::test]
async fn estimate_is_never_enforced() {
    let fixture = Fixture::new("r04-estimate-cap").await;
    let (ledger, child) = child_view(&fixture, "agent-r04-cap", "invocation-r04-cap", 2).await;

    let mut admitted = 0_u64;
    let (first_permit, first_attempt) = admitted_attempt(&child).await;
    admitted += 1;
    child
        .settle(first_permit, partial_settlement(first_attempt))
        .await
        .unwrap();

    let (_second_permit, _second_attempt) = admitted_attempt(&child).await;
    admitted += 1;

    let third_request = child.admit_model_request().await.unwrap();
    let third_attempt = PhysicalAttempt::new(third_request, EffectCallKind::Answer, 0);
    let denied = child
        .admit_physical_attempt(third_attempt)
        .await
        .unwrap_err();
    assert!(matches!(denied, EffectAdmissionError::Denied));
    assert_eq!(admitted, 2);

    let snapshot = ledger.snapshot().await;
    assert_eq!(snapshot.transport_attempts, 2);
    assert_eq!(snapshot.estimated_input_tokens, Some(10));
    assert_eq!(snapshot.estimated_output_tokens, Some(20));

    fixture.close().await;
}
