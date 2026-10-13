//! A job with ONE allowed write path gets exactly one child per run, however
//! often the model calls `delegate`: `coding.max_children` bounds simultaneous
//! children only, and a model that keeps delegating burned the whole request
//! cap in live trials. The owner refuses the extra delegations with a message
//! the model can act on, before any child record, checkout or budget exists.

use super::provider::Provider;
use super::support::Fixture;
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingJobRequest};
use saya_types::JobId;
use serde_json::Value;
use std::time::Duration;

const REFUSAL: &str = "one allowed write path";

/// The `delegate` tool results the model received, in call order: taken from
/// the captured request that carries the longest tool history.
fn delegate_results(provider: &Provider) -> Vec<String> {
    provider
        .bodies
        .lock()
        .unwrap()
        .iter()
        .filter_map(|body| serde_json::from_str::<Value>(body).ok())
        .map(|request| {
            request["messages"]
                .as_array()
                .map(|messages| {
                    messages
                        .iter()
                        .filter(|message| {
                            message["role"].as_str() == Some("tool")
                                && message["tool_call_id"].as_str() == Some("call-delegate")
                        })
                        .map(|message| message["content"].as_str().unwrap().to_owned())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
        .max_by_key(Vec::len)
        .expect("the provider captured requests")
}

async fn run_one_path(
    label: &str,
    provider: &Provider,
) -> (Fixture, std::sync::Arc<saya_cli::NativeCodingJob>) {
    let fixture = Fixture::with_endpoint(label, Some(&provider.endpoint)).await;
    let request = CodingJobRequest::new(
        JobId::parse(label).unwrap(),
        "Fix the one allowed file".into(),
        vec!["only.txt".into()],
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
    tokio::time::timeout(Duration::from_secs(30), job.run(&NoopEventSink))
        .await
        .unwrap()
        .unwrap();
    (fixture, job)
}

async fn one_child_and_one_checkout(fixture: &Fixture, job: &saya_cli::NativeCodingJob) {
    let children = fixture
        .store
        .list_coding_children(job.job_id(), None, 10)
        .await
        .unwrap();
    assert_eq!(children.items.len(), 1, "exactly one child record exists");
    let checkouts = fixture
        .store
        .list_managed_checkouts(job.job_id(), None, 10)
        .await
        .unwrap();
    assert_eq!(checkouts.items.len(), 1, "exactly one checkout exists");
}

#[tokio::test]
async fn two_tasks_in_one_delegate_call_are_refused_and_a_single_child_still_runs() {
    let provider = Provider::one_path();
    let (fixture, job) = run_one_path("one-child-two-tasks", &provider).await;
    let results = delegate_results(&provider);
    assert_eq!(
        results.len(),
        2,
        "two delegate calls were made: {results:?}"
    );
    assert!(
        results[0].contains(REFUSAL) && results[0].contains("refused"),
        "two tasks for one path are refused as data the model can correct: {}",
        results[0]
    );
    assert!(
        results[1].contains("child-one") && !results[1].contains(REFUSAL),
        "the single child is accepted (positive control): {}",
        results[1]
    );
    one_child_and_one_checkout(&fixture, &job).await;
}

#[tokio::test]
async fn a_second_delegation_after_the_join_is_refused_for_a_one_path_job() {
    let provider = Provider::one_path_again();
    let (fixture, job) = run_one_path("one-child-again", &provider).await;
    let results = delegate_results(&provider);
    assert_eq!(
        results.len(),
        2,
        "two delegate calls were made: {results:?}"
    );
    assert!(
        results[0].contains("child-one") && !results[0].contains(REFUSAL),
        "the first child is accepted (positive control): {}",
        results[0]
    );
    assert!(
        results[1].contains(REFUSAL) && results[1].contains("refused"),
        "a second delegation after the join is refused as data: {}",
        results[1]
    );
    one_child_and_one_checkout(&fixture, &job).await;
}
