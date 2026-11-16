//! The job-wide budget owner under the direct coordinator: the shared job
//! deadline stops a held provider. The job carries no write paths, so it owns
//! no working checkout W and keeps the coordinator-only tool surface.
use super::{provider::Provider, support::Fixture};
use saya_agent::{BudgetDimension, CancellationReason, CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingJobRequest};
use saya_types::JobId;

#[tokio::test]
async fn held_root_provider_is_cancelled_at_shared_job_deadline() {
    let provider = Provider::held();
    let fixture = Fixture::with_deadline("root-deadline", &provider.endpoint).await;
    let request = CodingJobRequest::new(
        JobId::parse("root-deadline").unwrap(),
        "Wait for the provider".into(),
        vec![],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), job.run(&NoopEventSink))
        .await
        .expect("job deadline interrupts the held request before provider timeout");
    // A controlled run converts the shared deadline into a graceful budget
    // stop, not an error: the latched Budget(ElapsedTime) reason surfaces as
    // `Limit("answering budget stopped")` (loop admission), which the runner
    // reports as an `Ok` output with `truncated` set.
    let output = result.expect("the shared job deadline stops the held run gracefully");
    assert!(
        output.truncated,
        "the elapsed-time budget stop marks the output truncated"
    );
    assert!(
        provider.calls.load(std::sync::atomic::Ordering::Acquire) > 0,
        "provider was actually entered"
    );
    assert_eq!(
        job.cancellation().reason(),
        Some(CancellationReason::Budget(BudgetDimension::ElapsedTime))
    );
}
