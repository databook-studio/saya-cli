//! Native-loop capability boundaries with real SQLite and managed checkouts.
#![cfg(unix)]
#[path = "coding_capabilities/provider.rs"]
#[allow(dead_code)] // shared fixture: this file uses only part of it
mod provider;
#[path = "coding_application/sqlite_fixture.rs"]
mod sqlite_fixture;
#[path = "coding_application/support.rs"]
#[allow(dead_code)]
mod support;

use provider::Provider;
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingJobRequest};
use saya_types::{JobId, ManagedCheckoutId};
use serde_json::Value;
use sha2::{Digest, Sha256};
use support::Fixture;
#[path = "coding_capabilities/database.rs"]
mod database;
#[path = "coding_capabilities/integration.rs"]
mod integration;

fn results(provider: &Provider, name: &str) -> Vec<String> {
    provider
        .bodies
        .lock()
        .unwrap()
        .iter()
        .flat_map(|body| {
            let value: Value = serde_json::from_str(body).unwrap();
            value["messages"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|m| {
                    m["role"] == "tool"
                        && m["tool_call_id"]
                            .as_str()
                            .is_some_and(|id| id.contains(name))
                })
                .map(|m| m["content"].as_str().unwrap_or("").to_owned())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The candidate the owner prepared at the end of the run: the only
/// `candidate-` managed checkout the job holds.
async fn prepared_candidate(fixture: &Fixture, job: &JobId) -> ManagedCheckoutId {
    fixture
        .store
        .list_managed_checkouts(job, None, 8)
        .await
        .unwrap()
        .items
        .iter()
        .map(|record| record.ownership.managed_checkout_id().clone())
        .find(|id| id.as_str().starts_with("candidate-"))
        .unwrap_or_else(|| panic!("the owner prepared a candidate at the end of the run"))
}

#[tokio::test]
async fn coordinator_inspects_and_reviews_owned_candidate_without_applying() {
    let provider = Provider::inspect_review();
    let mut fixture = Fixture::with_endpoint("inspection", Some(&provider.endpoint)).await;
    fixture.runtime.resolved.ai.context_byte_budget = 64 * 1024;
    // The consent snapshot is the repository as it stood when the job was
    // approved; a later edit to the repository must not reach the root.
    let approved_original = format!("original{}\n", "x".repeat(40 * 1024));
    std::fs::write(fixture.repository.join("tracked.txt"), &approved_original).unwrap();
    let request = CodingJobRequest::new(
        JobId::parse("inspection").unwrap(),
        "Inspect the working checkout and prepare a candidate".into(),
        vec!["tracked.txt".into()],
    )
    .unwrap()
    .with_tools_allow(Some(vec![
        "list".to_owned(),
        "read".to_owned(),
        "write".to_owned(),
    ]))
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    std::fs::write(
        fixture.repository.join("tracked.txt"),
        "live-original-changed\n",
    )
    .unwrap();
    let output = tokio::time::timeout(std::time::Duration::from_secs(30), job.run(&NoopEventSink))
        .await
        .unwrap()
        .unwrap();
    let reads = results(&provider, "inspect-read");
    let read: Value = serde_json::from_str(reads.last().unwrap()).unwrap();
    assert!(
        read.get("error").is_none(),
        "the root reads its own working checkout: {read}"
    );
    assert_eq!(read["path"], "tracked.txt");
    assert_eq!(read["size"], approved_original.len());
    assert_eq!(
        read["digest"],
        format!("{:x}", Sha256::digest(approved_original.as_bytes())),
        "inspection is bound to the consent snapshot, not the live repository"
    );
    let content = read["content"].as_str().unwrap();
    let content = content
        .strip_prefix("     1→")
        .expect("the read numbers its one line");
    assert!(approved_original.starts_with(content) && !content.is_empty());
    assert!(
        !content.contains("live-original-changed"),
        "a later repository edit never reaches the working checkout: {read}"
    );
    assert!(
        results(&provider, "inspect-list")
            .iter()
            .any(|r| r.contains("tracked.txt")),
        "the root lists its working checkout"
    );
    assert!(
        results(&provider, "inspect-missing")
            .iter()
            .any(|r| r.contains("error")),
        "a missing path is an error"
    );
    assert!(
        results(&provider, "inspect-escape")
            .iter()
            .any(|r| r.contains("error")),
        "a path outside the checkout is refused"
    );
    assert!(output.tool_metadata.iter().any(|tool| {
        let arguments: Value = serde_json::from_str(&tool.arguments).unwrap();
        tool.name == "read"
            && arguments.get("checkout").is_none()
            && arguments["path"] == "tracked.txt"
            && tool.status == "completed"
    }));
    assert_eq!(
        std::fs::read_to_string(fixture.repository.join("tracked.txt")).unwrap(),
        "live-original-changed\n",
        "the run never writes the repository"
    );
    // The owner prepared the candidate from the root's own working checkout at
    // the end of the run; reviewing it is the operator's step, and the run
    // advertises no `coding_apply` tool at all.
    let candidate = prepared_candidate(&fixture, job.job_id()).await;
    let proposal = fixture
        .application
        .propose_resume(job.job_id())
        .await
        .unwrap();
    let approval = CodingAuthorization::scoped(&proposal).unwrap();
    let resumed = fixture
        .application
        .resume(job.job_id(), &approval, CancellationToken::new())
        .await
        .unwrap();
    let review = resumed
        .review_existing(&candidate)
        .await
        .unwrap()
        .preview()
        .clone();
    assert!(
        review["source_sha256"]
            .as_str()
            .is_some_and(|digest| digest.len() == 64)
            && review["changes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|change| change["path"] == "tracked.txt"),
        "the operator review binds the changed file to a source digest: {review}"
    );
    assert_eq!(
        review["diff"]["files"][0]["after"], "candidate-only-content\n",
        "the reviewed candidate carries the root's change: {review}"
    );
    for body in provider.bodies.lock().unwrap().iter() {
        let body: Value = serde_json::from_str(body).unwrap();
        assert!(
            !body["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["function"]["name"] == "coding_apply"),
            "the run advertises no apply tool"
        );
    }
}
