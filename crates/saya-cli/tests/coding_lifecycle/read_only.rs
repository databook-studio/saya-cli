//! A job with no approved write path is read-only, not blind: it advertises
//! the read-only workspace tools, reads its own folder, and refuses writes.

use super::provider::Provider;
use super::support::Fixture;
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingJobRequest};
use saya_types::JobId;
use serde_json::Value;
use std::time::Duration;

const RUN: Duration = Duration::from_secs(60);

fn advertised(body: &str) -> Vec<String> {
    let request: Value = serde_json::from_str(body).expect("the provider captured a request");
    request["tools"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|tool| {
            tool["function"]["name"]
                .as_str()
                .or_else(|| tool["name"].as_str())
                .map(str::to_owned)
        })
        .collect()
}

async fn run_read_only(
    fixture: &Fixture,
    provider: &Provider,
    id: &str,
) -> (saya_agent::AgentOutput, Vec<String>) {
    let request = CodingJobRequest::new(
        JobId::parse(id).unwrap(),
        "What does tracked.txt say?".into(),
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
    let output = tokio::time::timeout(RUN, job.run(&NoopEventSink))
        .await
        .expect("the scripted run terminates")
        .unwrap();
    let bodies = provider.bodies.lock().unwrap().clone();
    (output, bodies)
}

fn assert_read_tools_and_no_writes(bodies: &[String]) {
    assert!(
        !bodies.is_empty(),
        "the provider captured at least one request"
    );
    let names = advertised(&bodies[0]);
    for name in ["read", "glob", "grep"] {
        assert!(names.contains(&name.to_string()), "advertised: {names:?}");
    }
    for name in ["write", "edit", "delete", "notebook_edit", "shell"] {
        assert!(
            !names.contains(&name.to_string()),
            "{name} must not be advertised: {names:?}"
        );
    }
    assert!(
        bodies.len() > 1,
        "the read result reached a later request: {bodies:?}"
    );
    assert!(
        bodies[1].contains("original"),
        "the read result reached the next request: {}",
        bodies[1]
    );
}

#[tokio::test]
async fn a_read_only_git_job_reads_its_folder_and_answers_from_it() {
    let provider = Provider::read_only();
    let fixture = Fixture::with_endpoint("read-only-git", Some(&provider.endpoint)).await;
    let (output, bodies) = run_read_only(&fixture, &provider, "read-only-git").await;
    assert!(
        output.answer.contains("original"),
        "the answer comes from the file: {}",
        output.answer
    );
    assert_read_tools_and_no_writes(&bodies);
}

#[tokio::test]
async fn a_read_only_in_place_job_reads_its_folder_and_answers_from_it() {
    let provider = Provider::read_only();
    let fixture = Fixture::in_place_with_endpoint("read-only-in-place", &provider.endpoint).await;
    let (output, bodies) = run_read_only(&fixture, &provider, "read-only-in-place").await;
    assert!(
        output.answer.contains("original"),
        "the answer comes from the file: {}",
        output.answer
    );
    assert_read_tools_and_no_writes(&bodies);
    assert!(!fixture.repository.join(".git").exists());
}
