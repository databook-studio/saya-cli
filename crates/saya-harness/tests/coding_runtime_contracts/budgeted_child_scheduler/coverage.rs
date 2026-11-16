use std::time::Duration;

use saya_agent::{
    EffectAdmission, EffectCallKind, EffectOutcome, EffectPermit, EffectSettlement,
    PhysicalAttempt, TokenUsage,
};
use saya_harness::coding::budget::{
    ChildBudgetView, ChildCountLimits, JobBudgetLedger, UsageCoverage,
};

use super::fixture::{Fixture, limits};

fn reported_settlement(attempt: PhysicalAttempt) -> EffectSettlement {
    EffectSettlement::PhysicalAttempt {
        attempt,
        outcome: EffectOutcome::Succeeded,
        reported_usage: Some(TokenUsage::new(10, 20)),
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

struct Overlap {
    ledger: JobBudgetLedger,
    child: ChildBudgetView,
    first_permit: EffectPermit,
    first_attempt: PhysicalAttempt,
    second_permit: EffectPermit,
    second_attempt: PhysicalAttempt,
}

/// Two logical requests, each holding one unsettled physical send at the same
/// time, so one attempt is always pending while the other settles.
async fn overlapping_attempts(fixture: &Fixture, agent: &str, invocation: &str) -> Overlap {
    let authority = fixture.add_child(agent, invocation).await;
    let ledger = fixture
        .ledger(limits(4, 4, 4, 2, 1, Duration::from_secs(30)))
        .await
        .unwrap();
    let session = ledger
        .begin_child(
            authority.agent_id(),
            authority.invocation_id(),
            ChildCountLimits {
                logical_requests: 2,
                tool_calls: 1,
            },
        )
        .await
        .unwrap();
    let child = session.view().clone();
    let first = child.admit_model_request().await.unwrap();
    let second = child.admit_model_request().await.unwrap();
    let first_attempt = PhysicalAttempt::new(first, EffectCallKind::Answer, 0);
    let second_attempt = PhysicalAttempt::new(second, EffectCallKind::Answer, 0);
    let first_permit = child.admit_physical_attempt(first_attempt).await.unwrap();
    let second_permit = child.admit_physical_attempt(second_attempt).await.unwrap();
    Overlap {
        ledger,
        child,
        first_permit,
        first_attempt,
        second_permit,
        second_attempt,
    }
}

#[tokio::test]
async fn settling_overlapping_fully_reported_attempts_reaches_complete_coverage() {
    let fixture = Fixture::new("coverage-complete").await;
    let overlap = overlapping_attempts(&fixture, "agent-coverage", "invocation-coverage").await;
    overlap
        .child
        .settle(
            overlap.first_permit,
            reported_settlement(overlap.first_attempt),
        )
        .await
        .unwrap();
    assert_eq!(
        overlap.ledger.snapshot().await.usage_coverage,
        UsageCoverage::Partial
    );
    overlap
        .child
        .settle(
            overlap.second_permit,
            reported_settlement(overlap.second_attempt),
        )
        .await
        .unwrap();
    assert_eq!(
        overlap.ledger.snapshot().await.usage_coverage,
        UsageCoverage::Complete
    );
    fixture.close().await;
}

#[tokio::test]
async fn one_unknown_attempt_keeps_coverage_from_ever_reaching_complete() {
    let fixture = Fixture::new("coverage-unknown").await;
    let overlap = overlapping_attempts(
        &fixture,
        "agent-coverage-unknown",
        "invocation-coverage-unknown",
    )
    .await;
    overlap
        .child
        .settle(
            overlap.first_permit,
            unknown_settlement(overlap.first_attempt),
        )
        .await
        .unwrap();
    assert_eq!(
        overlap.ledger.snapshot().await.usage_coverage,
        UsageCoverage::Unknown
    );
    overlap
        .child
        .settle(
            overlap.second_permit,
            reported_settlement(overlap.second_attempt),
        )
        .await
        .unwrap();
    assert_ne!(
        overlap.ledger.snapshot().await.usage_coverage,
        UsageCoverage::Complete
    );
    fixture.close().await;
}
