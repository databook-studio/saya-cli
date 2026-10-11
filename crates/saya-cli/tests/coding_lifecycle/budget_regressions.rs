use super::{provider::Provider, support::Fixture};
use saya_agent::{BudgetDimension, CancellationReason, CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingJobRequest};
use saya_types::{JobId, ManagedCheckoutId};

#[tokio::test]
async fn held_root_provider_is_cancelled_at_shared_job_deadline() {
    let provider = Provider::held();
    let fixture = Fixture::with_deadline("root-deadline", &provider.endpoint).await;
    let request = CodingJobRequest::new(
        JobId::parse("root-deadline").unwrap(),
        "Wait for the provider".into(),
        vec![],
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
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), job.run(&NoopEventSink))
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

#[tokio::test]
async fn active_check_blocks_cleanup_and_interrupted_check_blocks_recovery() {
    let fixture = Fixture::with_slow_check("check-reservation").await;
    let request = CodingJobRequest::new(
        JobId::parse("check-reservation").unwrap(),
        "Run the owned check".into(),
        vec![],
        vec!["sleep".into()],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    let checkout = ManagedCheckoutId::parse("checked-candidate").unwrap();
    job.create_checkout(&checkout).await.unwrap();
    let running = std::sync::Arc::clone(&job);
    let candidate = checkout.clone();
    let review = tokio::spawn(async move { running.review_existing(&candidate).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let reservations = fixture
                .store
                .list_coding_budget_reservations(job.job_id(), None, 100)
                .await
                .unwrap();
            if reservations.items.iter().any(|record| {
                record.settlement.is_none()
                    && record.reservation.charges().iter().any(|charge| {
                        charge.dimension() == saya_types::BudgetDimension::CommandSlots
                    })
            }) {
                break;
            }
            assert!(
                !review.is_finished(),
                "check must enter durable command admission"
            );
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        job.remove_checkout(&checkout).await.is_err(),
        "live command reservation excludes cleanup quiescence"
    );
    review.abort();
    let _ = review.await;
    let recovery = fixture
        .store
        .coding_recovery_status(job.job_id())
        .await
        .unwrap();
    assert!(recovery.unresolved_reservations > 0);
    assert!(!recovery.clear_for_resume());
    let approval = CodingAuthorization::scoped(
        &fixture
            .application
            .propose_resume(job.job_id())
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(
        fixture
            .application
            .resume(job.job_id(), &approval, CancellationToken::new())
            .await
            .is_err()
    );
    job.cancel();
}
