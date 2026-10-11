//! Resume tests for the conservative fold: charged counts, unknown
//! counters, consumed slot charges, and the next event sequence rebuild
//! exactly from durable records, and history over the bounded scan fails
//! closed.

use std::time::Duration;

use saya_agent::{
    EffectAdmission, EffectAdmissionError, EffectCallKind, EffectOutcome, EffectSettlement,
    LocalStateEffect, PhysicalAttempt, ToolEffect,
};
use saya_harness::coding::budget::{ChildCountLimits, JobBudgetLedger, UsageCoverage};
use saya_types::{BudgetEffectState, CodingEventKind, JobState};

use super::support::{Fixture, child_event, limits};

#[tokio::test]
async fn resume_rebuilds_charged_counts_and_next_sequence() {
    let fixture = Fixture::new("resume-rebuild-counts").await;
    let authority = fixture
        .add_child("agent-resume-a", "invocation-resume-a")
        .await;
    let owner_limits = limits(8, 8, 8, 4, 4, Duration::from_secs(30));
    let ledger = fixture.ledger(owner_limits).await.unwrap();
    let session = ledger
        .begin_child(
            authority.agent_id(),
            authority.invocation_id(),
            ChildCountLimits {
                logical_requests: 4,
                tool_calls: 4,
            },
        )
        .await
        .unwrap();
    let child = session.view().clone();
    let model = child.admit_model_request().await.unwrap();
    let attempt = PhysicalAttempt::new(model, EffectCallKind::Answer, 0);
    let send = child.admit_physical_attempt(attempt).await.unwrap();
    child
        .settle(
            send,
            EffectSettlement::PhysicalAttempt {
                attempt,
                outcome: EffectOutcome::Succeeded,
                reported_usage: None,
                usage_complete: false,
            },
        )
        .await
        .unwrap();
    child
        .settle(
            model,
            EffectSettlement::ModelRequest {
                outcome: EffectOutcome::Succeeded,
                reported_usage: None,
                usage_complete: false,
            },
        )
        .await
        .unwrap();
    let effect = ToolEffect {
        database_data: false,
        external_side_effect: false,
        requires_approval: false,
        local_state: LocalStateEffect::None,
    };
    let tools = child.admit_tool_batch(&[effect, effect]).await.unwrap();
    child
        .settle(
            tools[0],
            EffectSettlement::Tool {
                outcome: EffectOutcome::Succeeded,
            },
        )
        .await
        .unwrap();
    child
        .settle(
            tools[1],
            EffectSettlement::Tool {
                outcome: EffectOutcome::Succeeded,
            },
        )
        .await
        .unwrap();
    ledger
        .finish_child(session, EffectOutcome::Succeeded)
        .await
        .unwrap();
    let before = ledger.snapshot().await;
    assert_eq!(before.logical_requests, 1);
    assert_eq!(before.transport_attempts, 1);
    assert_eq!(before.tool_calls, 2);
    assert_eq!(before.model_slots_active, 0);
    assert_eq!(before.child_slots_active, 0);
    assert_eq!(before.measured_tool_calls, 2);
    fixture.close_store().await;
    drop(ledger);

    let store = fixture.reopen();
    let resumed = JobBudgetLedger::resume(store.clone(), fixture.job.clone(), owner_limits)
        .await
        .unwrap();
    let snapshot = resumed.snapshot().await;
    assert_eq!(snapshot.logical_requests, 1);
    assert_eq!(snapshot.transport_attempts, 1);
    assert_eq!(snapshot.tool_calls, 2);
    assert_eq!(snapshot.model_slots_active, 0);
    assert_eq!(snapshot.child_slots_active, 0);
    assert_eq!(snapshot.unknown_logical_requests, 0);
    assert_eq!(snapshot.unknown_transport_attempts, 0);
    assert_eq!(snapshot.unknown_tool_calls, 0);
    assert_eq!(snapshot.measured_tool_calls, 2);
    assert_eq!(snapshot.usage_coverage, UsageCoverage::Partial);
    assert!(!snapshot.poisoned);
    // The next sequencer allocation is exactly last + 1: the durable
    // history ends at sequence 9 (one running event, four reservations,
    // four settlements).
    let actor = (authority.agent_id(), authority.invocation_id());
    let appended = resumed
        .sequencer()
        .append_event(|sequence| {
            child_event(
                &fixture.job,
                actor,
                sequence,
                CodingEventKind::StateChanged {
                    state: JobState::Running,
                },
            )
        })
        .await
        .unwrap();
    assert_eq!(appended.sequence.get(), 10);
    let page = store
        .list_coding_events(&fixture.job, None, 100)
        .await
        .unwrap();
    let sequences: Vec<u64> = page
        .events
        .iter()
        .map(|event| event.sequence().get())
        .collect();
    assert_eq!(sequences, (1..=10).collect::<Vec<u64>>());
    fixture.close().await;
}

#[tokio::test]
async fn resume_unknown_or_unsettled_keeps_charged_and_slots_consumed() {
    let fixture = Fixture::new("resume-unknown-unsettled").await;
    let authority = fixture
        .add_child("agent-resume-b", "invocation-resume-b")
        .await;
    let owner_limits = limits(8, 8, 8, 2, 1, Duration::from_secs(30));
    let ledger = fixture.ledger(owner_limits).await.unwrap();
    let session = ledger
        .begin_child(
            authority.agent_id(),
            authority.invocation_id(),
            ChildCountLimits {
                logical_requests: 4,
                tool_calls: 4,
            },
        )
        .await
        .unwrap();
    let child = session.view().clone();
    let model = child.admit_model_request().await.unwrap();
    let _never_settled = ledger.root_view().admit_model_request().await.unwrap();
    let attempt = PhysicalAttempt::new(model, EffectCallKind::Answer, 0);
    let send = child.admit_physical_attempt(attempt).await.unwrap();
    // Cancelled after dispatch: the attempt settles Uncertain.
    ledger.cancellation_token().cancel();
    child
        .settle(
            send,
            EffectSettlement::PhysicalAttempt {
                attempt,
                outcome: EffectOutcome::Unknown,
                reported_usage: None,
                usage_complete: false,
            },
        )
        .await
        .unwrap();
    let before = ledger.snapshot().await;
    assert_eq!(before.logical_requests, 2);
    assert_eq!(before.transport_attempts, 1);
    assert_eq!(before.unknown_transport_attempts, 1);
    assert_eq!(before.model_slots_active, 2);
    assert_eq!(before.child_slots_active, 1);
    fixture.close_store().await;
    drop(ledger);

    let store = fixture.reopen();
    let resumed = JobBudgetLedger::resume(store.clone(), fixture.job.clone(), owner_limits)
        .await
        .unwrap();
    let snapshot = resumed.snapshot().await;
    assert_eq!(snapshot.logical_requests, 2);
    assert_eq!(snapshot.transport_attempts, 1);
    assert_eq!(snapshot.unknown_transport_attempts, 1);
    // unsettled model reservations are unknown after a restart (R04b)
    assert_eq!(snapshot.unknown_logical_requests, 2);
    assert_eq!(snapshot.unknown_tool_calls, 0);
    assert_eq!(snapshot.model_slots_active, 2);
    assert_eq!(snapshot.child_slots_active, 1);
    assert_eq!(snapshot.usage_coverage, UsageCoverage::Unknown);
    // The consumed pool caps admission: charged model slots at ceiling two.
    let denied = resumed.root_view().admit_model_request().await;
    assert!(matches!(denied, Err(EffectAdmissionError::Denied)));
    // The folded child slot is consumed: no new child begins after restart.
    let child_denied = resumed
        .begin_child(
            authority.agent_id(),
            authority.invocation_id(),
            ChildCountLimits {
                logical_requests: 4,
                tool_calls: 4,
            },
        )
        .await;
    assert!(matches!(child_denied, Err(EffectAdmissionError::Denied)));
    // The fold leaves the durable records untouched: four reservations —
    // the child slot from `begin_child`, both model requests, and the
    // physical attempt — of which three were never settled: the child run
    // was never finished, so its slot record stays settlement-less and
    // its charge retained as consumed capacity.
    let reservations = store
        .list_coding_budget_reservations(&fixture.job, None, 100)
        .await
        .unwrap();
    assert_eq!(reservations.items.len(), 4);
    assert_eq!(
        reservations
            .items
            .iter()
            .filter(|row| row.settlement.is_none())
            .count(),
        3
    );
    assert_eq!(
        reservations
            .items
            .iter()
            .filter(|row| row.settlement.as_ref().is_some_and(|settlement| {
                settlement.effect_state() == BudgetEffectState::Uncertain
            }))
            .count(),
        1
    );
    fixture.close().await;
}
