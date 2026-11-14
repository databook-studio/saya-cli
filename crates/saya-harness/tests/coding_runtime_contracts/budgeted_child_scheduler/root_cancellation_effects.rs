use super::{
    child_scheduler::{caps, finish, ledger, start},
    fixture::Fixture,
    lifecycle_support::{self, Behavior},
    openai_server::{LocalOpenAi, TOOL_RESPONSE},
};
use saya_agent::CancellationReason;
use saya_harness::coding::coordinator::{ChildRunError, ScheduleChildError, ScheduleStop};
use saya_types::{BudgetDimension, CodingResultOutcome};
use std::time::Duration;

#[tokio::test]
async fn panic_and_uncooperative_tool_leave_durable_unknown_reservations() {
    for (label, behavior) in [
        ("panic", Behavior::Panic),
        ("ignore", Behavior::IgnoreCancellation),
    ] {
        let fixture = Fixture::new(label).await;
        let ledger = ledger(&fixture).await;
        let mut server = LocalOpenAi::new().await;
        let (child, entered) = lifecycle_support::node(&fixture, &server, "agent-a", behavior).await;
        let control = child.control();
        let running = start(vec![child], caps(), ledger.clone());
        assert_eq!(server.request().await, Some(1));
        server.respond_sse(TOOL_RESPONSE);
        tokio::time::timeout(Duration::from_secs(2), entered.notified())
            .await
            .unwrap();
        if label == "ignore" {
            control.cancel();
        }
        let report = finish(running).await;
        assert_eq!(report.stop, Some(ScheduleStop::Unsettled));
        assert!(report.effects_uncertain);
        assert!(matches!(
            &report.outcomes[0].result,
            Err(ScheduleChildError::Join(_))
                | Err(ScheduleChildError::Run(ChildRunError::Unsettled))
        ));
        assert_eq!(
            ledger.cancellation_token().reason(),
            Some(CancellationReason::OwnerFailure)
        );
        let completions = fixture
            .store
            .list_coding_completions(&fixture.job, None, 32)
            .await
            .unwrap();
        assert_eq!(completions.items.len(), 1);
        assert_eq!(
            completions.items[0].result.outcome(),
            CodingResultOutcome::Uncertain
        );
        let reservations = fixture
            .store
            .list_coding_budget_reservations(&fixture.job, None, 100)
            .await
            .unwrap();
        assert!(reservations.items.iter().any(|record| {
            record.settlement.is_none()
                && record
                    .reservation
                    .charges()
                    .iter()
                    .any(|charge| charge.dimension() == BudgetDimension::ChildSlots)
        }));
        assert_eq!(ledger.snapshot().await.child_slots_active, 1);
        server.shutdown().await;
        fixture.close().await;
    }
}
