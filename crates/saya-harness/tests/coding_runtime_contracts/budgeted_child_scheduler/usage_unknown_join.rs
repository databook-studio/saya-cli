//! A cut model stream leaves only the usage of that attempt unknown. It must
//! stay visible in the ledger and the durable records, but it is not an
//! unknown effect: the join, the child and the recovery gate stay settled.
use super::{
    child_scheduler::{caps, finish, ledger, start},
    child_support::{FactoryStats, OpenAiFactory},
    fixture::Fixture,
    openai_server::{ANSWER_RESPONSE, LocalOpenAi},
};
use saya_harness::coding::{budget::UsageCoverage, coordinator::ScheduledChild};
use saya_types::{BudgetDimension, BudgetEffectState, BudgetObservationValue};
use std::{sync::Arc, time::Duration};

/// A child whose provider is allowed exactly one retry. The turn-level retry
/// count is the length of the configured schedule, so the shared helper (no
/// schedule, no retry) cannot recover from a cut.
pub(super) async fn child_with_one_retry(
    fixture: &Fixture,
    server: &LocalOpenAi,
) -> ScheduledChild {
    let authority = fixture.add_child("cut-child", "invocation-cut-child").await;
    let spec = fixture.spec(authority, "complete assigned task", 2, 1);
    let factory = OpenAiFactory {
        endpoint: server.endpoint.clone(),
        database: fixture.database.clone(),
        job: fixture.job.clone(),
        stats: Arc::new(FactoryStats::default()),
        cancel_on_build: None,
        retry_delays: vec![Duration::from_millis(5)],
    };
    ScheduledChild::new(spec, vec![], "provider".to_owned(), Arc::new(factory)).unwrap()
}

/// An SSE body that ends before its terminator, having delivered nothing: the
/// client cannot tell what the provider did, so the attempt settles with
/// unknown usage and is retried. It delivers nothing on purpose — a stream end
/// *after* content is a stream cut, which continues instead of retrying.
pub(super) const CUT_RESPONSE: &str = "data: {\"choices\":[{\"delta\":{}}]}\n\n";

#[tokio::test]
async fn a_cut_attempt_retried_to_completion_is_not_an_unknown_effect() {
    let fixture = Fixture::new("usage-unknown-join").await;
    let ledger = ledger(&fixture).await;
    let mut server = LocalOpenAi::new().await;
    let child = child_with_one_retry(&fixture, &server).await;
    let running = start(vec![child], caps(), ledger.clone());
    assert_eq!(server.request().await, Some(1));
    server.respond_sse(CUT_RESPONSE);
    assert_eq!(server.request().await, Some(2));
    server.respond_sse(ANSWER_RESPONSE);
    let report = finish(running).await;

    assert!(report.stop.is_none());
    assert!(report.outcomes[0].result.is_ok(), "the retry completed");
    assert!(
        !report.effects_uncertain,
        "a cut model stream cannot change the workspace"
    );

    let snapshot = ledger.snapshot().await;
    assert_eq!(
        snapshot.transport_attempts, 2,
        "the cut attempt still counts"
    );
    assert_eq!(snapshot.unknown_transport_attempts, 1);
    assert_eq!(snapshot.unknown_logical_requests, 0);
    assert_eq!(snapshot.unknown_tool_calls, 0);
    assert_eq!(snapshot.usage_coverage, UsageCoverage::Unknown);
    assert_eq!(snapshot.reported_input_tokens, None, "no known-zero usage");
    assert_eq!(snapshot.reported_output_tokens, None);
    assert_eq!(snapshot.child_slots_active, 0);
    assert_eq!(snapshot.model_slots_active, 0);

    let records = fixture
        .store
        .list_coding_budget_reservations(&fixture.job, None, 100)
        .await
        .unwrap();
    let attempts = records
        .items
        .iter()
        .filter(|record| {
            record
                .reservation
                .charges()
                .iter()
                .any(|charge| charge.dimension() == BudgetDimension::TransportAttempts)
        })
        .map(|record| record.settlement.as_ref().expect("attempt settled"))
        .collect::<Vec<_>>();
    assert_eq!(attempts.len(), 2);
    let unknown = attempts
        .iter()
        .filter(|settlement| {
            settlement.observations().iter().any(|observation| {
                observation.dimension() == BudgetDimension::TransportAttempts
                    && observation.value() == BudgetObservationValue::Unknown
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(unknown.len(), 1, "the cut attempt stays visibly unknown");
    assert_eq!(
        unknown[0].effect_state(),
        BudgetEffectState::Settled,
        "unknown usage is not an unreconciled effect"
    );
    let recovery = fixture
        .store
        .coding_recovery_status(&fixture.job)
        .await
        .unwrap();
    assert_eq!(recovery.unresolved_reservations, 0);
    server.shutdown().await;
    fixture.close().await;
}
