//! Budget-stop classification preserves remote uncertainty at the native scheduler owner.
use super::{
    child_scheduler::{caps, finish, ledger, node, start},
    fixture::Fixture,
    openai_server::LocalOpenAi,
};
use saya_agent::CancellationReason;
use std::sync::Arc;

#[tokio::test]
async fn scheduler_preserves_budget_stop_category_in_join_and_durable_completion() {
    use saya_agent::BudgetDimension;
    use saya_harness::coding::coordinator::classify_child_failure;
    use saya_types::CodingFailureClass;
    let fixture = Fixture::new("scheduler-budget-category").await;
    let ledger = ledger(&fixture).await;
    let mut server = LocalOpenAi::new().await;
    let child = node(
        &fixture,
        &server,
        "budget-child",
        vec![],
        "provider",
        Arc::default(),
    )
    .await;
    let running = start(vec![child], caps(), ledger.clone());
    assert_eq!(server.request().await, Some(1));
    ledger
        .cancellation_token()
        .cancel_with_reason(CancellationReason::Budget(BudgetDimension::ElapsedTime));
    let report = finish(running).await;
    let error = report.outcomes[0].result.as_ref().unwrap_err();
    assert_eq!(
        classify_child_failure(error),
        Some(CodingFailureClass::BudgetStopped)
    );
    let durable = fixture
        .store
        .list_coding_completions(&fixture.job, None, 32)
        .await
        .unwrap();
    assert_eq!(
        durable.items[0].result.failure_class(),
        Some(CodingFailureClass::BudgetStopped)
    );
    assert_eq!(ledger.snapshot().await.child_slots_active, 0);
    let records = fixture
        .store
        .list_coding_budget_reservations(&fixture.job, None, 100)
        .await
        .unwrap();
    let unknown = records
        .items
        .iter()
        .filter(|record| {
            record.settlement.as_ref().is_none_or(|settlement| {
                settlement.observations().iter().any(|observation| {
                    observation.value() == saya_types::BudgetObservationValue::Unknown
                })
            })
        })
        .collect::<Vec<_>>();
    // The request entered the provider but its response never arrived. A
    // joined budget stop keeps that remote uncertainty without refund, but a
    // model request cannot change the workspace: the request and its send
    // settle with unknown usage, the child has no unknown effect, and the
    // request's slot is free again.
    assert!(!report.effects_uncertain);
    assert_eq!(ledger.snapshot().await.model_slots_active, 0);
    assert_eq!(unknown.len(), 2);
    for record in &unknown {
        assert_eq!(
            record.reservation.agent_id(),
            Some(&report.outcomes[0].agent_id)
        );
        assert_eq!(
            record.reservation.invocation_id(),
            Some(&report.outcomes[0].invocation_id)
        );
        assert_eq!(
            record.settlement.as_ref().unwrap().effect_state(),
            saya_types::BudgetEffectState::Settled
        );
        assert!(
            record
                .reservation
                .charges()
                .iter()
                .all(|charge| charge.reserved().get() == 1)
        );
    }
    let dimensions = unknown
        .iter()
        .map(|record| {
            record
                .reservation
                .charges()
                .iter()
                .map(|charge| charge.dimension())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        dimensions,
        vec![
            vec![
                saya_types::BudgetDimension::LogicalRequests,
                saya_types::BudgetDimension::ModelRequestSlots
            ],
            vec![saya_types::BudgetDimension::TransportAttempts]
        ]
    );
    let recovery = fixture
        .store
        .coding_recovery_status(&fixture.job)
        .await
        .unwrap();
    assert_eq!(recovery.unresolved_reservations, 0);
    assert!(recovery.clear_for_resume());
    server.shutdown().await;
    fixture.close().await;
}
