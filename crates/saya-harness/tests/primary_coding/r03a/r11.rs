use super::{
    fixture::Fixture,
    openai_server::LocalOpenAi,
    scheduler::{caps, finish, ledger, node, start},
};
use saya_agent::CancellationReason;
use saya_harness::coding::coordinator::ScheduleStop;
use saya_types::CodingResultOutcome;

#[tokio::test]
async fn root_cancel_joins_all_started_children_and_persists_cancellation() {
    let fixture = Fixture::new("r11-root-cancel").await;
    let ledger = ledger(&fixture).await;
    let mut a = LocalOpenAi::new().await;
    let mut b = LocalOpenAi::new().await;
    let nodes = vec![
        node(
            &fixture,
            &a,
            "agent-a",
            "src",
            vec![],
            "provider-a",
            Default::default(),
        )
        .await,
        node(
            &fixture,
            &b,
            "agent-b",
            "docs",
            vec![],
            "provider-b",
            Default::default(),
        )
        .await,
    ];
    let running = start(nodes, caps(), ledger.clone());
    assert_eq!(a.request().await, Some(1));
    assert_eq!(b.request().await, Some(1));
    running.caller.cancel();
    let report = finish(running).await;
    assert_eq!(
        report.stop,
        Some(ScheduleStop::Cancelled(CancellationReason::User))
    );
    assert_eq!(report.outcomes.len(), 2);
    assert!(
        report
            .outcomes
            .iter()
            .all(|item| item.completion_sequence.is_some())
    );
    let results = fixture
        .store
        .list_coding_completions(&fixture.job, None, 32)
        .await
        .unwrap();
    assert_eq!(results.items.len(), 2);
    assert!(
        results
            .items
            .iter()
            .all(|item| item.result.outcome() == CodingResultOutcome::Cancelled)
    );
    let budget = ledger.snapshot().await;
    assert_eq!(budget.child_slots_active, 0);
    assert_eq!(budget.command_slots_active, 0);
    a.shutdown().await;
    b.shutdown().await;
    fixture.close().await;
}
