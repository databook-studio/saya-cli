//! R04b: unknown usage survives a restart — reservations left without a
//! settlement fold to unknown coverage with their charges and slots
//! retained, a second resume of the same durable history is idempotent,
//! and settled known usage does not become unknown.

use std::time::Duration;

use saya_agent::{
    EffectAdmission, EffectAdmissionError, EffectCallKind, EffectOutcome, EffectSettlement,
    PhysicalAttempt, TokenUsage,
};
use saya_harness::coding::budget::{
    ChildBudgetView, ChildCountLimits, CountLimits, JobBudgetLedger, UsageCoverage,
};
use saya_store::SqliteStateStore;

use super::fixture::{Fixture, limits};

/// Owner limits under test: a transport cap of 1 and a logical cap of 2,
/// with generous tool and slot ceilings so only those caps bind.
fn owner_limits() -> CountLimits {
    limits(2, 1, 4, 4, 2, Duration::from_secs(30))
}

/// Begins one child against [`owner_limits`] and returns its ledger and
/// the child's budget view.
async fn begun_child(
    fixture: &Fixture,
    agent: &str,
    invocation: &str,
) -> (JobBudgetLedger, ChildBudgetView) {
    let authority = fixture.add_child(agent, invocation).await;
    let ledger = fixture.ledger(owner_limits()).await.unwrap();
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
    (ledger, child)
}

/// Admits a model request and a physical send over it and leaves both
/// reservations unsettled, as a child mid-flight would at process death.
async fn unsettled_ledger(fixture: &Fixture) -> JobBudgetLedger {
    let (ledger, child) = begun_child(fixture, "agent-r04-restart", "invocation-r04-restart").await;
    let request = child.admit_model_request().await.unwrap();
    let attempt = PhysicalAttempt::new(request, EffectCallKind::Answer, 0);
    child.admit_physical_attempt(attempt).await.unwrap();
    ledger
}

#[tokio::test]
async fn unknown_usage_survives_restart() {
    let fixture = Fixture::new("r04-restart-unknown").await;
    let ledger = unsettled_ledger(&fixture).await;
    let fresh = fixture
        .add_child("agent-r04-restart-b", "invocation-r04-restart-b")
        .await;
    fixture.store.close().await;
    drop(ledger);

    let store = SqliteStateStore::new(&fixture.database);
    let resumed = JobBudgetLedger::resume(store, fixture.job.clone(), owner_limits())
        .await
        .unwrap();
    let snapshot = resumed.snapshot().await;
    assert_eq!(snapshot.logical_requests, 1);
    assert_eq!(snapshot.transport_attempts, 1);
    assert_eq!(snapshot.model_slots_active, 1);
    assert_eq!(snapshot.unknown_transport_attempts, 1);
    assert_eq!(snapshot.unknown_logical_requests, 1);
    assert_eq!(snapshot.usage_coverage, UsageCoverage::Unknown);
    // By construction exactly two reservations were left without a
    // settlement: one logical (the model request) and one transport (the
    // physical send); the fold counts both as unsettled.
    assert_eq!(snapshot.unsettled_reservations, 2);

    // The allowance is not reset: the transport cap of 1 is already
    // consumed by the unsettled attempt, so a fresh child may begin, but
    // its physical admission is refused.
    let session = resumed
        .begin_child(
            fresh.agent_id(),
            fresh.invocation_id(),
            ChildCountLimits {
                logical_requests: 2,
                tool_calls: 1,
            },
        )
        .await
        .unwrap();
    let child = session.view().clone();
    let request = child.admit_model_request().await.unwrap();
    let attempt = PhysicalAttempt::new(request, EffectCallKind::Answer, 0);
    let denied = child.admit_physical_attempt(attempt).await.unwrap_err();
    assert!(matches!(denied, EffectAdmissionError::Denied));
    fixture.close().await;
}

#[tokio::test]
async fn second_resume_is_idempotent() {
    let fixture = Fixture::new("r04-restart-idempotent").await;
    let ledger = unsettled_ledger(&fixture).await;
    fixture.store.close().await;
    drop(ledger);

    let store = SqliteStateStore::new(&fixture.database);
    let resumed = JobBudgetLedger::resume(store.clone(), fixture.job.clone(), owner_limits())
        .await
        .unwrap();
    let first = resumed.snapshot().await;
    store.close().await;
    drop(resumed);

    let store = SqliteStateStore::new(&fixture.database);
    let resumed = JobBudgetLedger::resume(store, fixture.job.clone(), owner_limits())
        .await
        .unwrap();
    let second = resumed.snapshot().await;
    assert_eq!(second, first);
    fixture.close().await;
}

#[tokio::test]
async fn settled_known_attempt_is_not_unknown_after_restart() {
    let fixture = Fixture::new("r04-restart-settled").await;
    let (ledger, child) =
        begun_child(&fixture, "agent-r04-settled", "invocation-r04-settled").await;
    let request = child.admit_model_request().await.unwrap();
    let attempt = PhysicalAttempt::new(request, EffectCallKind::Answer, 0);
    let permit = child.admit_physical_attempt(attempt).await.unwrap();
    child
        .settle(
            permit,
            EffectSettlement::PhysicalAttempt {
                attempt,
                outcome: EffectOutcome::Succeeded,
                reported_usage: Some(TokenUsage::new(3, 4)),
                usage_complete: true,
            },
        )
        .await
        .unwrap();
    child
        .settle(
            request,
            EffectSettlement::ModelRequest {
                outcome: EffectOutcome::Succeeded,
                reported_usage: Some(TokenUsage::new(3, 4)),
                usage_complete: true,
            },
        )
        .await
        .unwrap();
    fixture.store.close().await;
    drop(ledger);

    let store = SqliteStateStore::new(&fixture.database);
    let resumed = JobBudgetLedger::resume(store, fixture.job.clone(), owner_limits())
        .await
        .unwrap();
    let snapshot = resumed.snapshot().await;
    assert_eq!(snapshot.unsettled_reservations, 0);
    assert_eq!(snapshot.unknown_transport_attempts, 0);
    assert_ne!(snapshot.usage_coverage, UsageCoverage::Unknown);
    fixture.close().await;
}
