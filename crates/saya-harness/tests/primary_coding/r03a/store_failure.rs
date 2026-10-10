use std::{sync::Arc, time::Duration};

use saya_agent::{CancellationReason, CancellationToken};
use saya_harness::coding::coordinator::{ChildRunError, child_event_channel, run_child_budgeted};

use super::{
    child_support::{FactoryStats, OpenAiFactory},
    fixture::{Fixture, limits},
    openai_server::LocalOpenAi,
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
async fn failed_durable_child_reservation_denies_factory_and_provider() {
    let fixture = Fixture::new("store-unavailable").await;
    let authority = fixture
        .add_child("agent-store-down", "invocation-store-down")
        .await;
    let mut server = LocalOpenAi::new().await;
    let ledger = fixture
        .ledger(limits(2, 2, 1, 1, 1, Duration::from_secs(5)))
        .await
        .unwrap();
    let spec = fixture.spec(authority, "must not dispatch", 2, 1);
    let (sender, _receiver) = child_event_channel(&spec);
    let stats = Arc::new(FactoryStats::default());
    let child_factory = factory(&server, &fixture, Arc::clone(&stats));
    fixture.store.close().await;
    let result = run_child_budgeted(
        spec,
        &child_factory,
        sender,
        CancellationToken::new(),
        &ledger,
    )
    .await;
    assert!(matches!(result, Err(ChildRunError::OwnerFailure)));
    assert_eq!(stats.builds.load(Ordering::SeqCst), 0);
    assert_eq!(stats.executor_calls.load(Ordering::SeqCst), 0);
    assert_eq!(server.count.load(Ordering::SeqCst), 0);
    assert!(ledger.snapshot().await.poisoned);
    assert_eq!(
        ledger.cancellation_token().reason(),
        Some(CancellationReason::OwnerFailure)
    );
    server.shutdown().await;
    fixture.close().await;
}
