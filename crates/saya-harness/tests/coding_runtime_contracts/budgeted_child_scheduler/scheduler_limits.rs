//! R08: normal no-start release and independent capacity refusal controls.
use super::{
    child_scheduler::{caps, finish, ledger, node, start},
    child_support::FactoryStats,
    fixture::{Fixture, limits},
    openai_server::{ANSWER_RESPONSE, LocalOpenAi},
};
use saya_agent::{CancellationToken, EffectOutcome};
use saya_harness::coding::{
    budget::ChildCountLimits,
    coordinator::{ScheduleError, ScheduleLimits, run_schedule},
};
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;

#[tokio::test]
async fn active_child_no_start_refusal_releases_schedule_without_owner_failure() {
    let fixture = Fixture::new("r08-no-start").await;
    let ledger = ledger(&fixture).await;
    let active = fixture.add_child("agent-active", "invocation-active").await;
    let session = ledger
        .begin_child(
            active.agent_id(),
            active.invocation_id(),
            ChildCountLimits {
                logical_requests: 1,
                tool_calls: 1,
            },
        )
        .await
        .unwrap();
    let mut server = LocalOpenAi::new().await;
    let pending = node(
        &fixture,
        &server,
        "agent-pending",
        "src",
        vec![],
        "provider",
        Arc::default(),
    )
    .await;
    let (sender, _receiver) = mpsc::channel(32);
    assert!(matches!(
        run_schedule(
            vec![pending],
            caps(),
            &ledger,
            CancellationToken::new(),
            sender
        )
        .await,
        Err(ScheduleError::ActiveChildren)
    ));
    assert_eq!(ledger.cancellation_token().reason(), None);
    ledger
        .finish_child(session, EffectOutcome::NotStarted)
        .await
        .unwrap();
    let next = node(
        &fixture,
        &server,
        "agent-next",
        "docs",
        vec![],
        "provider",
        Arc::default(),
    )
    .await;
    let running = start(vec![next], caps(), ledger.clone());
    assert_eq!(server.request().await, Some(1));
    server.respond_sse(ANSWER_RESPONSE);
    assert!(finish(running).await.stop.is_none());
    assert_eq!(ledger.cancellation_token().reason(), None);
    server.shutdown().await;
    fixture.close().await;
}

#[tokio::test]
async fn model_headroom_and_provider_caps_are_independently_refused_before_effects() {
    let fixture = Fixture::new("r08-capacity-guards").await;
    // Child slots allow three, but two model slots leave only one child headroom.
    let ledger = fixture
        .ledger(limits(12, 12, 8, 2, 3, Duration::from_secs(20)))
        .await
        .unwrap();
    let server = LocalOpenAi::new().await;
    let stats = Arc::new(FactoryStats::default());
    for (index, limits) in [
        ScheduleLimits {
            max_active_children: 2,
            max_active_per_provider: 1,
        },
        ScheduleLimits {
            max_active_children: 1,
            max_active_per_provider: 2,
        },
        ScheduleLimits {
            max_active_children: 0,
            max_active_per_provider: 1,
        },
    ]
    .into_iter()
    .enumerate()
    {
        let child = node(
            &fixture,
            &server,
            &format!("agent-{index}"),
            "src",
            vec![],
            "provider",
            stats.clone(),
        )
        .await;
        let (sender, _receiver) = mpsc::channel(32);
        assert!(matches!(
            run_schedule(
                vec![child],
                limits,
                &ledger,
                CancellationToken::new(),
                sender
            )
            .await,
            Err(ScheduleError::InvalidLimits)
        ));
    }
    assert_eq!(stats.builds.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert!(
        fixture
            .store
            .list_coding_budget_reservations(&fixture.job, None, 100)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(ledger.cancellation_token().reason(), None);
    server.shutdown().await;
    fixture.close().await;
}
