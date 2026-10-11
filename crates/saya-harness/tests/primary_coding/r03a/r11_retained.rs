#[cfg(unix)]
use super::openai_server::TOOL_RESPONSE;
use super::{
    fixture::Fixture,
    lifecycle_support::{self, Behavior},
    openai_server::{ANSWER_RESPONSE, LocalOpenAi},
    scheduler::{caps, finish, ledger, start},
};
use saya_harness::coding::coordinator::{ChildRunError, ScheduleChildError, ScheduleStop};
use saya_types::CodingResultOutcome;

#[tokio::test]
async fn failed_resource_settlement_cannot_publish_success_or_release_child_slot() {
    let fixture = Fixture::new("settlement-failure").await;
    let ledger = ledger(&fixture).await;
    let mut server = LocalOpenAi::new().await;
    let (child, _) = lifecycle_support::node(
        &fixture,
        &ledger,
        &server,
        "agent-a",
        Behavior::SettlementFailure,
    )
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

#[cfg(unix)]
#[tokio::test]
async fn natural_completion_settles_retained_process_before_persisting_result() {
    use super::lifecycle_support::process;
    let Some(live) = process::fixture() else {
        return;
    };
    let fixture = Fixture::new("retained-command").await;
    let ledger = ledger(&fixture).await;
    let mut server = LocalOpenAi::new().await;
    let (parent, child_marker) = live.marker_pair("retained-command");
    let (child, _) = lifecycle_support::node(
        &fixture,
        &ledger,
        &server,
        "agent-a",
        Behavior::Command {
            live,
            parent: parent.clone(),
            child: child_marker.clone(),
            retain: true,
        },
    )
    .await;
    let running = start(vec![child], caps(), ledger.clone());
    assert_eq!(server.request().await, Some(1));
    server.respond_sse(TOOL_RESPONSE);
    let pid = process::wait_for_file(&parent).await;
    let descendant = process::wait_for_file(&child_marker).await;
    assert_eq!(server.request().await, Some(2));
    assert!(process::process_is_alive(pid));
    assert_eq!(ledger.snapshot().await.command_slots_active, 1);
    server.respond_sse(ANSWER_RESPONSE);
    let report = finish(running).await;
    assert!(report.stop.is_none());
    assert!(report.outcomes[0].completion_sequence.is_some());
    assert!(!process::process_is_alive(descendant));
    assert!(process::wait_group_gone(pid).await);
    let snapshot = ledger.snapshot().await;
    assert_eq!(snapshot.command_slots_active, 0);
    assert_eq!(snapshot.child_slots_active, 0);
    server.shutdown().await;
    fixture.close().await;
}
