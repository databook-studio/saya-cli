use super::{
    child_scheduler::{caps, finish, ledger, start},
    fixture::Fixture,
    lifecycle_support::{self, Behavior},
    openai_server::{ANSWER_RESPONSE, LocalOpenAi},
};
use saya_harness::coding::coordinator::{ChildRunError, ScheduleChildError, ScheduleStop};
use saya_types::CodingResultOutcome;

#[tokio::test]
async fn failed_resource_settlement_cannot_publish_success_or_release_child_slot() {
    let fixture = Fixture::new("settlement-failure").await;
    let ledger = ledger(&fixture).await;
    let mut server = LocalOpenAi::new().await;
    let (child, _) = lifecycle_support::node(&fixture, &server, "agent-a", Behavior::SettlementFailure)
        .await;
    let running = start(vec![child], caps(), ledger.clone());
    assert_eq!(server.request().await, Some(1));
    server.respond_sse(ANSWER_RESPONSE);
    let report = finish(running).await;
    assert_eq!(report.stop, Some(ScheduleStop::Unsettled));
    assert!(matches!(
        &report.outcomes[0].result,
        Err(ScheduleChildError::Run(ChildRunError::Settlement(_)))
    ));
    assert_eq!(ledger.snapshot().await.child_slots_active, 1);
    let durable = fixture
        .store
        .list_coding_completions(&fixture.job, None, 32)
        .await
        .unwrap();
    assert_eq!(
        durable.items[0].result.outcome(),
        CodingResultOutcome::Uncertain
    );
    server.shutdown().await;
    fixture.close().await;
}

#[tokio::test]
async fn cleanup_failure_retains_primary_child_provider_cause_with_uncertain_effects() {
    let fixture = Fixture::new("primary-and-cleanup-failure").await;
    let ledger = ledger(&fixture).await;
    let mut server = LocalOpenAi::new().await;
    let (child, _) = lifecycle_support::node(&fixture, &server, "agent-a", Behavior::SettlementFailure)
        .await;
    let running = start(vec![child], caps(), ledger.clone());
    assert_eq!(server.request().await, Some(1));
    server.respond_status(500);
    let report = finish(running).await;
    assert!(report.effects_uncertain);
    let detail = format!("{:?}", report.outcomes[0].result);
    assert!(
        detail.contains("Provider"),
        "cleanup hid the primary cause: {detail}"
    );
    assert_eq!(ledger.snapshot().await.child_slots_active, 1);
    let durable = fixture
        .store
        .list_coding_completions(&fixture.job, None, 32)
        .await
        .unwrap();
    assert_eq!(
        durable.items[0].result.outcome(),
        CodingResultOutcome::Uncertain
    );
    assert!(
        !String::from_utf8(durable.items[0].result.to_json_bytes().unwrap())
            .unwrap()
            .contains("provider unavailable")
    );
    server.shutdown().await;
    fixture.close().await;
}
