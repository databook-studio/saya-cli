//! R08: unavailable root progress must fail closed and still join child work.
use super::{
    fixture::Fixture,
    openai_server::{LocalOpenAi, TOOL_RESPONSE},
    scheduler::{caps, ledger, node},
};
use saya_agent::CancellationToken;
use saya_harness::coding::coordinator::{
    ChildRunError, ScheduleChildError, ScheduleStop, run_schedule,
};
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;

#[tokio::test]
async fn progress_owner_closed_after_provider_starts_cancels_and_joins_child() {
    let fixture = Fixture::new("r08-progress-closed").await;
    let ledger = ledger(&fixture).await;
    let mut server = LocalOpenAi::new().await;
    let child = node(
        &fixture,
        &server,
        "agent-progress",
        "src",
        vec![],
        "provider",
        Arc::default(),
    )
    .await;
    let (sender, receiver) = mpsc::channel(128);
    let child_ledger = ledger.clone();
    let running = tokio::spawn(async move {
        run_schedule(
            vec![child],
            caps(),
            &child_ledger,
            CancellationToken::new(),
            sender,
        )
        .await
    });
    assert_eq!(server.request().await, Some(1));
    // Close only after a real held provider request. Its tool response emits
    // new progress into the closed owner, forcing failure during started work.
    drop(receiver);
    server.respond_sse(TOOL_RESPONSE);
    let report = tokio::time::timeout(Duration::from_secs(5), running)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(report.stop, Some(ScheduleStop::ChildFailed)));
    assert_eq!(report.outcomes.len(), 1);
    assert!(report.outcomes[0].result.is_err());
    assert!(report.not_started.is_empty());
    assert_eq!(ledger.snapshot().await.child_slots_active, 0);
    server.shutdown().await;
    fixture.close().await;
}

#[tokio::test]
async fn full_progress_queue_cancels_without_waiting_for_root_capacity() {
    let fixture = Fixture::new("r08-progress-full").await;
    let ledger = ledger(&fixture).await;
    let server = LocalOpenAi::new().await;
    let child = node(
        &fixture,
        &server,
        "agent-progress",
        "src",
        vec![],
        "provider",
        Arc::default(),
    )
    .await;
    let (sender, _receiver) = mpsc::channel(1);
    let report = tokio::time::timeout(
        Duration::from_secs(5),
        run_schedule(
            vec![child],
            caps(),
            &ledger,
            CancellationToken::new(),
            sender,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(matches!(report.stop, Some(ScheduleStop::ChildFailed)));
    assert_eq!(
        _receiver.capacity(),
        0,
        "a real event saturated the root queue"
    );
    assert!(matches!(
        &report.outcomes[0].result,
        Err(ScheduleChildError::Run(ChildRunError::OwnerFailure))
    ));
    // A provider timeout must not satisfy the observer-failure proof.
    assert_eq!(ledger.cancellation_token().reason(), None);
    let settled = ledger.snapshot().await;
    assert!(!settled.poisoned);
    assert_eq!(settled.child_slots_active, 0);
    server.shutdown().await;
    fixture.close().await;
}
