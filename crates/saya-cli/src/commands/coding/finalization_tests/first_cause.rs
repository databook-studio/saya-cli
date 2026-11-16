use super::super::super::{CodingApplication, CodingError, NativeCodingJob};
use crate::coding_test_support::follow_up_fixture::Fixture;
use saya_types::{CodingEventKind, CodingFailureClass as Class, CodingRunFailure};
use std::{
    fs,
    sync::Arc,
    time::{Duration, Instant},
};
pub(super) async fn prior_error(job: &Arc<NativeCodingJob>, fixture: &Fixture) {
    let observation = job.records.join("observations.jsonl");
    fs::rename(
        &observation,
        job.records.join("observations.original-fixture.jsonl"),
    )
    .unwrap();
    let target = fixture
        .repository
        .parent()
        .unwrap()
        .join("diagnostic-target");
    fs::write(&target, b"untouched private target").unwrap();
    std::os::unix::fs::symlink(&target, &observation).unwrap();
    let lease = job.operation.lock().await;
    let failure = CodingRunFailure::root(Class::ProviderError);
    let finalizing = job.finalize_run(Err(CodingError::Failed(Box::new(failure.clone()))));
    tokio::pin!(finalizing);
    assert!(
        tokio::time::timeout(Duration::from_millis(300), &mut finalizing)
            .await
            .is_err()
    );
    assert!(
        job.record_observation_boundary("actual-owner-fault", false)
            .await
            .is_err()
    );
    drop(lease);
    let result = tokio::time::timeout(Duration::from_secs(10), &mut finalizing)
        .await
        .unwrap();
    assert!(
        matches!(result,Err(CodingError::Failed(ref actual)) if actual.as_ref()==&failure),
        "earlier supplied root failure must remain primary: {result:?}"
    );
    let event = fixture
        .store
        .latest_coding_run_event(job.job_id())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(event.event(),CodingEventKind::RunTerminated{failure:actual,secondary:Some(Class::DiagnosticStorage),diagnostic_fault:Some(_),..} if actual==&failure)
    );
    assert_eq!(fs::read(target).unwrap(), b"untouched private target");
}
pub(super) async fn waiting_cancel(
    job: &Arc<NativeCodingJob>,
    fixture: &Fixture,
    app: &CodingApplication,
) {
    let lease = job.operation.lock().await;
    let owner = Arc::clone(job);
    let output = fixture.output.clone();
    let mut finalizing =
        tokio::task::spawn_local(async move { owner.finalize_run(Ok(output)).await });
    assert!(
        tokio::time::timeout(Duration::from_millis(300), &mut finalizing)
            .await
            .is_err()
    );
    app.cancel(job.job_id()).await.unwrap();
    let wait = Instant::now();
    while !job.cancellation.is_cancelled() && wait.elapsed() < Duration::from_secs(2) {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let cancelled = job.cancellation.is_cancelled();
    drop(lease);
    let result = tokio::time::timeout(Duration::from_secs(10), &mut finalizing)
        .await
        .unwrap()
        .unwrap();
    assert!(
        cancelled,
        "actual finalizer monitor must notice durable cancel while waiting"
    );
    assert!(
        matches!(result,Err(CodingError::Failed(ref failure)) if failure.class==Class::Cancelled),
        "cancelled finalization requires the actual cause: {result:?}"
    );
    let event = fixture
        .store
        .latest_coding_run_event(job.job_id())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(event.event(),CodingEventKind::RunTerminated{failure,..} if failure.class==Class::Cancelled)
    );
}
