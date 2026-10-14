use super::{
    fixture::Fixture,
    lifecycle_support::{self, Behavior},
    openai_server::{LocalOpenAi, TOOL_RESPONSE},
    scheduler::{caps, finish, ledger, start},
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
        let (child, entered) =
            lifecycle_support::node(&fixture, &ledger, &server, "agent-a", behavior).await;
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

#[cfg(unix)]
#[tokio::test]
async fn cancel_one_then_root_joins_actual_owned_process_groups() {
    use super::lifecycle_support::process;
    let Some(live) = process::fixture() else {
        return;
    };
    let fixture = Fixture::new("r11-process-groups").await;
    let ledger = ledger(&fixture).await;
    let mut a = LocalOpenAi::new().await;
    let mut b = LocalOpenAi::new().await;
    let (a_parent, a_child) = live.marker_pair("r11-a");
    let (b_parent, b_child) = live.marker_pair("r11-b");
    let (first, _) = lifecycle_support::node(
        &fixture,
        &ledger,
        &a,
        "agent-a",
        Behavior::Command {
            retain: false,
            live,
            parent: a_parent.clone(),
            child: a_child.clone(),
        },
    )
    .await;
    let control = first.control();
    let (second, _) = lifecycle_support::node(
        &fixture,
        &ledger,
        &b,
        "agent-b",
        Behavior::Command {
            retain: false,
            live,
            parent: b_parent.clone(),
            child: b_child.clone(),
        },
    )
    .await;
    let running = start(vec![first, second], caps(), ledger.clone());
    assert_eq!(a.request().await, Some(1));
    assert_eq!(b.request().await, Some(1));
    a.respond_sse(TOOL_RESPONSE);
    b.respond_sse(TOOL_RESPONSE);
    let a_pid = process::wait_for_file(&a_parent).await;
    let a_child_pid = process::wait_for_file(&a_child).await;
    let b_pid = process::wait_for_file(&b_parent).await;
    let b_child_pid = process::wait_for_file(&b_child).await;
    control.cancel();
    assert!(process::wait_group_gone(a_pid).await);
    assert!(!process::process_is_alive(a_child_pid));
    assert!(process::process_is_alive(b_pid));
    assert!(process::process_is_alive(b_child_pid));
    running.caller.cancel();
    let report = finish(running).await;
    assert_eq!(
        report.stop,
        Some(ScheduleStop::Cancelled(CancellationReason::User))
    );
    assert!(process::wait_group_gone(b_pid).await);
    assert!(!process::process_is_alive(b_child_pid));
    let budget = ledger.snapshot().await;
    assert_eq!(budget.child_slots_active, 0);
    assert_eq!(budget.command_slots_active, 0);
    assert_eq!(budget.model_slots_active, 0);
    a.shutdown().await;
    b.shutdown().await;
    fixture.close().await;
}
