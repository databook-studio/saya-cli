//! The optional job-wide token ceiling reaches the job's budget owner: with `max_job_tokens` set,
//! a model request after the reported total passes it is denied; unset, the same run is unchanged.

use super::provider::Provider;
use super::support::Fixture;
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingApplication, CodingAuthorization, CodingJobRequest};
use saya_types::JobId;
use std::sync::Arc;
use std::time::Duration;

/// Every scripted provider response reports 10 prompt + 5 completion tokens.
const TOKENS_PER_RESPONSE: u64 = 15;

async fn application_with_ceiling(
    fixture: &Fixture,
    ceiling: Option<u64>,
) -> Arc<CodingApplication> {
    let mut runtime = fixture.runtime.clone();
    runtime.resolved.coding.max_job_tokens = ceiling;
    CodingApplication::open(
        runtime,
        fixture.store.clone(),
        &fixture.repository,
        &fixture.root.join("coding"),
        &fixture.git,
        &fixture.program_dir,
        &CancellationToken::new(),
    )
    .await
    .unwrap()
}

/// Runs the one-path script under `ceiling` and returns how many model requests the provider saw.
async fn requests_seen(label: &str, ceiling: Option<u64>) -> usize {
    let provider = Provider::one_path();
    let fixture = Fixture::with_endpoint(label, Some(&provider.endpoint)).await;
    let application = application_with_ceiling(&fixture, ceiling).await;
    let request = CodingJobRequest::new(
        JobId::parse(label).unwrap(),
        "Fix the one allowed file".into(),
        vec!["only.txt".into()],
        vec![],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&application.propose(&request).await.unwrap()).unwrap();
    let job = application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    // The run may end in an error when the ceiling stops it; only the request count matters here.
    let _ = tokio::time::timeout(Duration::from_secs(30), job.run(&NoopEventSink))
        .await
        .expect("the job ends instead of hanging");
    provider.bodies.lock().unwrap().len()
}

#[tokio::test]
async fn an_unset_job_ceiling_leaves_the_run_unchanged() {
    let seen = requests_seen("job-ceiling-unset", None).await;
    assert!(seen > 1, "the scripted run makes several requests: {seen}");
}

#[tokio::test]
async fn a_set_job_ceiling_denies_the_next_model_request_once_the_reported_total_reaches_it() {
    let baseline = requests_seen("job-ceiling-baseline", None).await;
    let capped = requests_seen("job-ceiling-capped", Some(TOKENS_PER_RESPONSE)).await;
    assert_eq!(capped, 1, "only the first request fits under the ceiling");
    assert!(capped < baseline, "baseline {baseline}, capped {capped}");
}
