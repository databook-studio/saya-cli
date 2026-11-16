//! A model request Saya gives up on (retries exhausted, or cancellation
//! settled) loses only its usage. It must stay visible in the ledger and the
//! durable records, but it is not an unknown effect: the child stays settled,
//! the join and recovery need no reconciliation, and the request's concurrency
//! slot is free again. An unknown tool still decides all of that.
use std::time::Duration;

use saya_agent::{
    EffectAdmission, EffectCallKind, EffectOutcome, EffectSettlement, LocalStateEffect,
    PhysicalAttempt, ToolEffect,
};
use saya_harness::coding::{
    budget::{ChildCountLimits, JobBudgetLedger, UsageCoverage},
    coordinator::{ScheduleStop, classify_child_failure},
};
use saya_types::{BudgetDimension, BudgetEffectState, BudgetObservationValue, CodingFailureClass};

use super::{
    child_scheduler::{caps, finish, ledger, start},
    fixture::{Fixture, limits},
    openai_server::LocalOpenAi,
    usage_unknown_join::{CUT_RESPONSE, child_with_one_retry},
};

#[tokio::test]
async fn an_exhausted_model_request_is_not_an_unknown_effect_and_frees_its_slot() {
    let fixture = Fixture::new("usage-unknown-exhausted").await;
    let ledger = ledger(&fixture).await;
    let mut server = LocalOpenAi::new().await;
    let child = child_with_one_retry(&fixture, &server).await;
    let running = start(vec![child], caps(), ledger.clone());
    // One retry is allowed, and both attempts are cut: the request is
    // exhausted and the child fails on the provider stream.
    assert_eq!(server.request().await, Some(1));
    server.respond_sse(CUT_RESPONSE);
    assert_eq!(server.request().await, Some(2));
    server.respond_sse(CUT_RESPONSE);
    let report = finish(running).await;

    let error = report.outcomes[0].result.as_ref().unwrap_err();
    // Every attempt ended before its terminator, which the loop reports as a
    // stream-ended provider failure.
    assert_eq!(
        classify_child_failure(error),
        Some(CodingFailureClass::ProviderStreamEnded)
    );
    assert_eq!(report.stop, Some(ScheduleStop::ChildFailed));
    assert!(
        !report.effects_uncertain,
        "a model request cannot change the workspace"
    );

    let snapshot = ledger.snapshot().await;
    assert_eq!(snapshot.logical_requests, 1);
    assert_eq!(snapshot.transport_attempts, 2, "both attempts still count");
    assert_eq!(snapshot.unknown_logical_requests, 1);
    assert_eq!(snapshot.unknown_transport_attempts, 2);
    assert_eq!(snapshot.unknown_tool_calls, 0);
    assert_eq!(snapshot.usage_coverage, UsageCoverage::Unknown);
    assert_eq!(snapshot.reported_input_tokens, None, "no known-zero usage");
    assert_eq!(snapshot.model_slots_active, 0, "the slot is free again");
    assert_eq!(snapshot.child_slots_active, 0);

    let records = fixture
        .store
        .list_coding_budget_reservations(&fixture.job, None, 100)
        .await
        .unwrap();
    let request = records
        .items
        .iter()
        .find(|record| {
            record
                .reservation
                .charges()
                .iter()
                .any(|charge| charge.dimension() == BudgetDimension::LogicalRequests)
        })
        .and_then(|record| record.settlement.as_ref())
        .expect("the request settled");
    assert!(request.observations().iter().any(|observation| {
        observation.dimension() == BudgetDimension::LogicalRequests
            && observation.value() == BudgetObservationValue::Unknown
    }));
    assert_eq!(request.effect_state(), BudgetEffectState::Settled);
    let recovery = fixture
        .store
        .coding_recovery_status(&fixture.job)
        .await
        .unwrap();
    assert_eq!(recovery.unresolved_reservations, 0);
    server.shutdown().await;
    fixture.close().await;
}

/// One child: a model request that is given up on with unknown usage, then an
/// optional tool call settling as `tool`.
async fn finished_child(fixture: &Fixture, tool: Option<EffectOutcome>) -> (JobBudgetLedger, bool) {
    let authority = fixture
        .add_child("agent-given-up", "invocation-given-up")
        .await;
    let ledger = fixture
        .ledger(limits(4, 8, 4, 2, 1, Duration::from_secs(30)))
        .await
        .unwrap();
    let session = ledger
        .begin_child(
            authority.agent_id(),
            authority.invocation_id(),
            ChildCountLimits {
                logical_requests: 3,
                tool_calls: 2,
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
                outcome: EffectOutcome::Unknown,
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
                outcome: EffectOutcome::Unknown,
                reported_usage: None,
                usage_complete: false,
            },
        )
        .await
        .unwrap();
    if let Some(outcome) = tool {
        let effect = ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::None,
        };
        let permits = child.admit_tool_batch(&[effect]).await.unwrap();
        child
            .settle(permits[0], EffectSettlement::Tool { outcome })
            .await
            .unwrap();
    }
    ledger
        .finish_child(session, EffectOutcome::Succeeded)
        .await
        .unwrap();
    let started = ledger
        .child_started(authority.agent_id(), authority.invocation_id())
        .await;
    (ledger, started)
}

#[tokio::test]
async fn a_request_given_up_on_leaves_the_child_settled_and_the_job_quiescent() {
    let fixture = Fixture::new("usage-unknown-given-up").await;
    let (ledger, unsettled) = finished_child(&fixture, None).await;
    assert!(!unsettled, "unknown usage is not an unsettled child");
    let snapshot = ledger.snapshot().await;
    assert_eq!(snapshot.unknown_logical_requests, 1);
    assert_eq!(snapshot.model_slots_active, 0);
    assert!(
        ledger.try_quiescent().await.is_ok(),
        "no held slot or unknown effect keeps the job from quiescing"
    );
    fixture.close().await;
}

#[tokio::test]
async fn a_request_given_up_on_never_masks_an_unknown_tool_effect() {
    let fixture = Fixture::new("usage-unknown-given-up-tool").await;
    let (ledger, unsettled) = finished_child(&fixture, Some(EffectOutcome::Unknown)).await;
    assert!(
        unsettled,
        "an unknown tool effect still leaves the child unsettled"
    );
    let snapshot = ledger.snapshot().await;
    assert_eq!(snapshot.unknown_tool_calls, 1);
    assert_eq!(snapshot.unknown_logical_requests, 1);
    fixture.close().await;
}
