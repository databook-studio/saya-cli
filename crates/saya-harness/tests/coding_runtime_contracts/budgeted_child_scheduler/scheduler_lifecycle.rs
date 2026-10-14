//! R08: whole-schedule ownership and ordinary no-start refusal.

use super::{
    child_support::FactoryStats,
    fixture::Fixture,
    openai_server::{ANSWER_RESPONSE, LocalOpenAi},
    scheduler::{caps, finish, ledger, node, start},
};
use saya_agent::{CancellationReason, CancellationToken};
use saya_harness::coding::coordinator::{ScheduleError, ScheduleLimits, run_schedule};
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;

#[tokio::test]
async fn cloned_ledger_refuses_overlapping_schedule_without_poisoning_first() {
    let fixture = Fixture::new("r08-owner-exclusion").await;
    let ledger = ledger(&fixture).await;
    let mut first_server = LocalOpenAi::new().await;
    let second_server = LocalOpenAi::new().await;
    let first = node(
        &fixture,
        &first_server,
        "agent-a",
        "src",
        vec![],
        "provider-a",
        Arc::default(),
    )
    .await;
    let second_stats = Arc::new(FactoryStats::default());
    let second = node(
        &fixture,
        &second_server,
        "agent-b",
        "src",
        vec![],
        "provider-b",
        second_stats.clone(),
    )
    .await;
    let running = start(vec![first], caps(), ledger.clone());
    assert_eq!(first_server.request().await, Some(1));
    let (sender, _receiver) = mpsc::channel(32);
    let denied = tokio::time::timeout(
        Duration::from_secs(1),
        run_schedule(
            vec![second],
            caps(),
            &ledger,
            CancellationToken::new(),
            sender,
        ),
    )
    .await
    .unwrap();
    assert!(matches!(denied, Err(ScheduleError::Busy)));
    assert_eq!(
        second_stats
            .builds
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert_eq!(ledger.cancellation_token().reason(), None);
    first_server.respond_sse(ANSWER_RESPONSE);
    assert!(finish(running).await.stop.is_none());
    first_server.shutdown().await;
    second_server.shutdown().await;
    fixture.close().await;
}

#[tokio::test]
async fn aborted_started_schedule_latches_owner_failure_and_refuses_retry() {
    let fixture = Fixture::new("r08-aborted").await;
    let ledger = ledger(&fixture).await;
    let mut server = LocalOpenAi::new().await;
    let first = node(
        &fixture,
        &server,
        "agent-first",
        "src",
        vec![],
        "provider",
        Arc::default(),
    )
    .await;
    let running = start(vec![first], caps(), ledger.clone());
    assert_eq!(server.request().await, Some(1));
    running.task.abort();
    assert!(running.task.await.unwrap_err().is_cancelled());
    assert_eq!(
        ledger.cancellation_token().reason(),
        Some(CancellationReason::OwnerFailure)
    );
    tokio::time::timeout(Duration::from_secs(5), running.progress)
        .await
        .unwrap()
        .unwrap();
    let stats = Arc::new(FactoryStats::default());
    let next = node(
        &fixture,
        &server,
        "agent-next",
        "docs",
        vec![],
        "provider",
        stats.clone(),
    )
    .await;
    let (sender, _receiver) = mpsc::channel(32);
    assert!(matches!(
        run_schedule(
            vec![next],
            caps(),
            &ledger,
            CancellationToken::new(),
            sender
        )
        .await,
        Err(ScheduleError::OwnerUnavailable)
    ));
    assert_eq!(stats.builds.load(std::sync::atomic::Ordering::SeqCst), 0);
    // This proves fail-closed admission, not settled reservations or crash recovery.
    server.shutdown().await;
    fixture.close().await;
}

#[tokio::test]
async fn provider_key_cap_blocks_disjoint_child_until_first_settles() {
    let fixture = Fixture::new("r08-provider-cap").await;
    let ledger = ledger(&fixture).await;
    let mut first_server = LocalOpenAi::new().await;
    let mut second_server = LocalOpenAi::new().await;
    let stats = Arc::new(FactoryStats::default());
    let first = node(
        &fixture,
        &first_server,
        "agent-first",
        "src",
        vec![],
        "provider",
        Arc::default(),
    )
    .await;
    let second = node(
        &fixture,
        &second_server,
        "agent-second",
        "docs",
        vec![],
        "provider",
        stats.clone(),
    )
    .await;
    let running = start(
        vec![first, second],
        ScheduleLimits {
            max_active_per_provider: 1,
            ..caps()
        },
        ledger.clone(),
    );
    assert_eq!(first_server.request().await, Some(1));
    assert_eq!(stats.builds.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(ledger.snapshot().await.child_slots_active, 1);
    first_server.respond_sse(ANSWER_RESPONSE);
    assert_eq!(second_server.request().await, Some(1));
    second_server.respond_sse(ANSWER_RESPONSE);
    assert!(finish(running).await.stop.is_none());
    first_server.shutdown().await;
    second_server.shutdown().await;
    fixture.close().await;
}

#[tokio::test]
async fn failed_ledger_is_not_reported_as_simultaneous_user_cancellation() {
    let fixture = Fixture::new("r08-owner-cancel-priority").await;
    let ledger = ledger(&fixture).await;
    let mut server = LocalOpenAi::new().await;
    let child = node(
        &fixture,
        &server,
        "agent-first",
        "src",
        vec![],
        "provider",
        Arc::default(),
    )
    .await;
    let running = start(vec![child], caps(), ledger.clone());
    assert_eq!(server.request().await, Some(1));
    // Latch both before yielding: the first caller cause must not hide owner failure.
    running.caller.cancel_with_reason(CancellationReason::User);
    ledger
        .cancellation_token()
        .cancel_with_reason(CancellationReason::OwnerFailure);
    let report = finish(running).await;
    assert_eq!(
        report.stop,
        Some(saya_harness::coding::coordinator::ScheduleStop::OwnerFailure)
    );
    assert_eq!(ledger.snapshot().await.child_slots_active, 0);
    server.shutdown().await;
    fixture.close().await;
}
