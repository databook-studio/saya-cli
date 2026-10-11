//! R07a: real native loops share job budgets without sharing child admission.

use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use saya_agent::{AgentOutput, CancellationToken};
use saya_harness::coding::{
    budget::JobBudgetLedger,
    child::ChildExecutionSpec,
    coordinator::{ChildRunError, child_event_channel, run_child_budgeted},
};
use tokio::task::JoinHandle;

use super::{
    child_support::{FactoryStats, OpenAiFactory},
    fixture::{Fixture, limits},
    openai_server::{ANSWER_RESPONSE, LocalOpenAi},
};

fn factory(server: &LocalOpenAi, fixture: &Fixture, stats: Arc<FactoryStats>) -> OpenAiFactory {
    OpenAiFactory {
        endpoint: server.endpoint.clone(),
        database: fixture.database.clone(),
        job: fixture.job.clone(),
        stats,
        cancel_on_build: None,
        retry_delays: vec![],
    }
}

fn start(
    spec: ChildExecutionSpec,
    factory: OpenAiFactory,
    ledger: JobBudgetLedger,
) -> JoinHandle<Result<AgentOutput, ChildRunError>> {
    let (sender, receiver) = child_event_channel(&spec);
    tokio::spawn(async move {
        // Keep this child's independent event sink alive for the whole invocation.
        let _receiver = receiver;
        run_child_budgeted(spec, &factory, sender, CancellationToken::new(), &ledger).await
    })
}

async fn finish(child: JoinHandle<Result<AgentOutput, ChildRunError>>) {
    let output = tokio::time::timeout(Duration::from_secs(5), child)
        .await
        .expect("child must finish")
        .unwrap()
        .unwrap();
    assert_eq!(output.answer, "completed");
}

#[tokio::test]
async fn duplicate_active_child_is_refused_without_cancelling_the_first() {
    let fixture = Fixture::new("r07-busy-child").await;
    let authority = fixture.add_child("agent-busy", "invocation-busy").await;
    let ledger = fixture
        .ledger(limits(4, 4, 2, 3, 3, Duration::from_secs(15)))
        .await
        .unwrap();
    let mut server = LocalOpenAi::new().await;
    let first_stats = Arc::new(FactoryStats::default());
    let first = start(
        fixture.spec(authority.clone(), "first turn", 2, 1),
        factory(&server, &fixture, first_stats.clone()),
        ledger.clone(),
    );
    assert_eq!(server.request().await, Some(1));
    let before = fixture
        .store
        .list_coding_budget_reservations(&fixture.job, None, 100)
        .await
        .unwrap();
    let duplicate_stats = Arc::new(FactoryStats::default());
    let duplicate = fixture.spec(authority, "second concurrent turn", 2, 1);
    let (sender, _receiver) = child_event_channel(&duplicate);
    let denied = tokio::time::timeout(
        Duration::from_secs(2),
        run_child_budgeted(
            duplicate,
            &factory(&server, &fixture, duplicate_stats.clone()),
            sender,
            CancellationToken::new(),
            &ledger,
        ),
    )
    .await
    .expect("busy refusal must not wait for first provider");
    assert!(matches!(denied, Err(ChildRunError::BudgetDenied)));
    assert_eq!(duplicate_stats.builds.load(Ordering::SeqCst), 0);
    assert_eq!(ledger.cancellation_token().reason(), None);
    let active = ledger.snapshot().await;
    assert!(!active.poisoned);
    assert_eq!(active.child_slots_active, 1);
    let after = fixture
        .store
        .list_coding_budget_reservations(&fixture.job, None, 100)
        .await
        .unwrap();
    assert_eq!(after.items.len(), before.items.len());
    server.respond_sse(ANSWER_RESPONSE);
    finish(first).await;
    assert_eq!(first_stats.builds.load(Ordering::SeqCst), 1);
    assert_eq!(server.count.load(Ordering::SeqCst), 1);
    assert_eq!(ledger.snapshot().await.child_slots_active, 0);
    server.shutdown().await;
    fixture.close().await;
}

#[tokio::test]
async fn distinct_native_children_reach_providers_before_either_answer_finishes() {
    let fixture = Fixture::new("r07-independent-children").await;
    let first_view = fixture.add_child("agent-first", "invocation-first").await;
    let second_view = fixture.add_child("agent-second", "invocation-second").await;
    let ledger = fixture
        .ledger(limits(4, 4, 2, 2, 2, Duration::from_secs(15)))
        .await
        .unwrap();
    let mut first_server = LocalOpenAi::new().await;
    let mut second_server = LocalOpenAi::new().await;
    let first = start(
        fixture.spec(first_view, "first child", 2, 1),
        factory(&first_server, &fixture, Arc::new(FactoryStats::default())),
        ledger.clone(),
    );
    assert_eq!(first_server.request().await, Some(1));
    let second = start(
        fixture.spec(second_view, "second child", 2, 1),
        factory(&second_server, &fixture, Arc::new(FactoryStats::default())),
        ledger.clone(),
    );
    // Separate servers can each hold an unanswered request; no sequential fake overlap.
    assert_eq!(second_server.request().await, Some(1));
    let active = ledger.snapshot().await;
    assert_eq!(active.child_slots_active, 2);
    assert_eq!(active.model_slots_active, 2);
    assert_eq!(active.logical_requests, 2);
    assert_eq!(active.transport_attempts, 2);
    first_server.respond_sse(ANSWER_RESPONSE);
    second_server.respond_sse(ANSWER_RESPONSE);
    tokio::join!(finish(first), finish(second));
    let done = ledger.snapshot().await;
    assert!(!done.poisoned);
    assert_eq!(done.child_slots_active, 0);
    assert_eq!(done.model_slots_active, 0);
    first_server.shutdown().await;
    second_server.shutdown().await;
    fixture.close().await;
}
