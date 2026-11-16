use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use saya_agent::{AgentError, CancellationToken};
use saya_harness::coding::coordinator::{ChildRunError, child_event_channel, run_child_budgeted};
use saya_store::SqliteStateStore;
use saya_types::BudgetDimension;

use super::{
    child_support::{FactoryStats, OpenAiFactory},
    fixture::{Fixture, limits},
    openai_server::{ANSWER_RESPONSE, LocalOpenAi, TWO_TOOL_RESPONSE},
};

fn factory(server: &LocalOpenAi, fixture: &Fixture, stats: Arc<FactoryStats>) -> OpenAiFactory {
    OpenAiFactory {
        endpoint: server.endpoint.clone(),
        database: fixture.database.clone(),
        job: fixture.job.clone(),
        stats,
        cancel_on_build: None,
        retry_delays: vec![Duration::from_millis(5)],
    }
}

#[tokio::test]
async fn public_budgeted_child_uses_durable_logical_physical_and_atomic_tool_admission() {
    let fixture = Fixture::new("public-child").await;
    let authority = fixture.add_child("agent-public", "invocation-public").await;
    let mut server = LocalOpenAi::new().await;
    let ledger = fixture
        .ledger(limits(3, 3, 2, 1, 1, Duration::from_secs(5)))
        .await
        .unwrap();
    let cancellation = CancellationToken::new();
    let spec = fixture.spec(authority, "inspect two things", 3, 2);
    let (sender, _receiver) = child_event_channel(&spec);
    let stats = Arc::new(FactoryStats::default());
    let child_factory = factory(&server, &fixture, Arc::clone(&stats));
    let child_ledger = ledger.clone();
    let child_cancellation = cancellation.clone();
    let child = tokio::spawn(async move {
        run_child_budgeted(
            spec,
            &child_factory,
            sender,
            child_cancellation,
            &child_ledger,
        )
        .await
    });

    assert_eq!(server.request().await, Some(1));
    let before_first_send = SqliteStateStore::new(fixture.database.clone())
        .list_coding_budget_reservations(&fixture.job, None, 10)
        .await
        .unwrap();
    assert!(before_first_send.items.iter().any(|record| {
        record
            .reservation
            .charges()
            .iter()
            .any(|charge| charge.dimension() == BudgetDimension::LogicalRequests)
            && record
                .reservation
                .charges()
                .iter()
                .any(|charge| charge.dimension() == BudgetDimension::ModelRequestSlots)
    }));
    assert!(before_first_send.items.iter().any(|record| {
        record
            .reservation
            .charges()
            .iter()
            .any(|charge| charge.dimension() == BudgetDimension::TransportAttempts)
    }));
    assert!(!before_first_send.items.iter().any(|record| {
        record
            .reservation
            .charges()
            .iter()
            .any(|charge| charge.dimension() == BudgetDimension::ToolCalls)
    }));
    server.respond_sse(TWO_TOOL_RESPONSE);

    assert_eq!(server.request().await, Some(2));
    let records = SqliteStateStore::new(fixture.database.clone())
        .list_coding_budget_reservations(&fixture.job, None, 20)
        .await
        .unwrap();
    let batch = records
        .items
        .iter()
        .filter_map(|record| {
            record
                .reservation
                .charges()
                .iter()
                .find(|charge| charge.dimension() == BudgetDimension::ToolCalls)
        })
        .collect::<Vec<_>>();
    assert_eq!(batch.len(), 1);
    assert_eq!(batch[0].reserved().get(), 2);
    server.respond_sse(ANSWER_RESPONSE);
    assert_eq!(child.await.unwrap().unwrap().answer, "completed");
    assert_eq!(server.count.load(Ordering::SeqCst), 2);
    assert_eq!(stats.executor_calls.load(Ordering::SeqCst), 2);
    assert!(stats.executor_saw_tool_reservation.load(Ordering::SeqCst));
    let snapshot = ledger.snapshot().await;
    assert_eq!(snapshot.logical_requests, 2);
    assert_eq!(snapshot.transport_attempts, 2);
    assert_eq!(snapshot.tool_calls, 2);
    assert_eq!(snapshot.child_slots_active, 0);
    assert_eq!(snapshot.model_slots_active, 0);
    server.shutdown().await;
    fixture.close().await;
}

#[tokio::test]
async fn denied_multi_tool_batch_starts_no_executor_and_sends_no_later_request() {
    let fixture = Fixture::new("batch-denial").await;
    let authority = fixture
        .add_child("agent-batch-deny", "invocation-batch-deny")
        .await;
    let mut server = LocalOpenAi::new().await;
    let ledger = fixture
        .ledger(limits(3, 3, 1, 1, 1, Duration::from_secs(5)))
        .await
        .unwrap();
    let spec = fixture.spec(authority, "inspect twice", 3, 2);
    let (sender, _receiver) = child_event_channel(&spec);
    let stats = Arc::new(FactoryStats::default());
    let child_factory = factory(&server, &fixture, Arc::clone(&stats));
    let child_ledger = ledger.clone();
    let child = tokio::spawn(async move {
        run_child_budgeted(
            spec,
            &child_factory,
            sender,
            CancellationToken::new(),
            &child_ledger,
        )
        .await
    });
    assert_eq!(server.request().await, Some(1));
    server.respond_sse(TWO_TOOL_RESPONSE);
    let result = child.await.unwrap();
    assert!(matches!(
        result,
        Err(ChildRunError::Agent(AgentError::EffectAdmissionDenied))
    ));
    assert_eq!(
        server.request_within(Duration::from_millis(100)).await,
        None
    );
    assert_eq!(server.count.load(Ordering::SeqCst), 1);
    assert_eq!(stats.executor_calls.load(Ordering::SeqCst), 0);
    assert!(
        !fixture
            .store
            .list_coding_budget_reservations(&fixture.job, None, 20)
            .await
            .unwrap()
            .items
            .iter()
            .any(|record| record
                .reservation
                .charges()
                .iter()
                .any(|charge| charge.dimension() == BudgetDimension::ToolCalls))
    );
    server.shutdown().await;
    fixture.close().await;
}

#[tokio::test]
async fn retry_denial_occurs_before_a_second_loopback_http_send() {
    let fixture = Fixture::new("retry-denial").await;
    let authority = fixture.add_child("agent-retry", "invocation-retry").await;
    let mut server = LocalOpenAi::new().await;
    let ledger = fixture
        .ledger(limits(2, 1, 1, 1, 1, Duration::from_secs(5)))
        .await
        .unwrap();
    let spec = fixture.spec(authority, "retry once", 2, 1);
    let (sender, _receiver) = child_event_channel(&spec);
    let stats = Arc::new(FactoryStats::default());
    let child_factory = factory(&server, &fixture, Arc::clone(&stats));
    let child_ledger = ledger.clone();
    let child = tokio::spawn(async move {
        run_child_budgeted(
            spec,
            &child_factory,
            sender,
            CancellationToken::new(),
            &child_ledger,
        )
        .await
    });
    assert_eq!(server.request().await, Some(1));
    server.respond_status(500);
    let result = child.await.unwrap();
    assert!(matches!(
        result,
        Err(ChildRunError::Agent(AgentError::EffectAdmissionDenied))
    ));
    assert_eq!(
        server.request_within(Duration::from_millis(100)).await,
        None
    );
    assert_eq!(server.count.load(Ordering::SeqCst), 1);
    let snapshot = ledger.snapshot().await;
    assert_eq!(snapshot.transport_attempts, 1);
    assert!(snapshot.unknown_transport_attempts >= 1);
    server.shutdown().await;
    fixture.close().await;
}
