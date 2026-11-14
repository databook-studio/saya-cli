#[path = "continuation_support.rs"]
mod continuation_support;

use super::{provider::Provider, support::Fixture};
use continuation_support::{
    CaptureProvider, append_completion_history, append_large_pending_tasks,
};
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingError, CodingJobRequest, CodingTask, NativeCodingJob};
use saya_types::{AgentId, JobId, ManagedCheckoutId};
use serde_json::json;
use std::{fs, io::Write, sync::Arc, sync::atomic::Ordering};

/// An approved job request over `write_paths`, with no programs.
fn direct_request(id: &str, task: &str, write_paths: Vec<String>) -> CodingJobRequest {
    CodingJobRequest::new(
        JobId::parse(id).unwrap(),
        format!("{task} with helpers"),
        write_paths,
    )
    .unwrap()
}

/// Continues the stored job after a fresh operator approval.
async fn reopen(fixture: &Fixture, job: &JobId) -> Arc<NativeCodingJob> {
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose_resume(job).await.unwrap())
            .unwrap();
    fixture
        .application
        .resume(job, &approval, CancellationToken::new())
        .await
        .unwrap()
}

#[tokio::test]
async fn continuation_context_compacts_large_history_without_losing_durable_identities() {
    let provider = CaptureProvider::new();
    let fixture = Fixture::with_endpoint("continuation-context", Some(&provider.endpoint)).await;
    let request = direct_request(
        "continuation-context",
        "Make dependent native child changes",
        vec![
            "shared.txt".into(),
            "rename-before.txt".into(),
            "rename-after.txt".into(),
        ],
    );
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    job.run(&NoopEventSink).await.unwrap();
    assert_eq!(
        fixture
            .store
            .list_coding_children(job.job_id(), None, 10)
            .await
            .unwrap()
            .items
            .len(),
        2,
        "the dependent pair ran (direct mode admits them on disjoint leases)"
    );
    let completion = fixture
        .store
        .list_coding_completion_history(job.job_id(), None, 32)
        .await
        .unwrap()
        .items
        .into_iter()
        .next()
        .unwrap();
    assert!(job.accept_finding(completion.sequence).await.unwrap());
    // A run that ended with a final answer is `completed`, and child lifecycle
    // events may only be appended while the job is resumable. Reopen it to
    // record the synthetic history, then reopen again so the run's sequencer
    // starts after that history instead of at the sequence it saw first.
    let reopened = reopen(&fixture, job.job_id()).await;
    let later_accepted = append_completion_history(&fixture, job.job_id()).await;
    assert!(job.accept_finding(later_accepted).await.unwrap());
    let first_page = fixture
        .store
        .list_coding_completion_history(job.job_id(), None, 32)
        .await
        .unwrap();
    let after = first_page.next_cursor;
    assert!(after.is_some());
    assert!(
        first_page
            .items
            .iter()
            .all(|item| item.sequence != later_accepted)
    );
    let second_page = fixture
        .store
        .list_coding_completion_history(job.job_id(), after, 32)
        .await
        .unwrap();
    assert!(second_page.next_cursor.is_none());
    let unaccepted_items: Vec<_> = first_page
        .items
        .iter()
        .chain(second_page.items.iter())
        .filter(|item| !item.acknowledged)
        .collect();
    let unaccepted = unaccepted_items.len();
    assert!(unaccepted > 0, "the replay needs at least one open result");
    let unaccepted_sequences: Vec<u64> = unaccepted_items
        .iter()
        .map(|item| item.sequence.get())
        .collect();
    append_large_pending_tasks(&fixture, job.job_id());
    drop(reopened);
    let resumed = reopen(&fixture, job.job_id()).await;

    // Direct-mode assembly sources its single candidate from the root working
    // checkout W, and an overlap is only ever detected BETWEEN accepted
    // deltas, so a direct job cannot refuse a candidate the way two
    // overlapping delegated children could.
    let candidate = ManagedCheckoutId::parse("continuation-conflict").unwrap();
    let reviewed = resumed.review(&candidate).await;
    assert!(
        reviewed.is_ok(),
        "the resumed direct owner assembles its one candidate from W: {:?}",
        reviewed.err()
    );
    let readiness = resumed.status().await.unwrap()["readiness"].clone();
    let output = resumed.run(&NoopEventSink).await.unwrap();
    assert!(!output.truncated);
    assert!(provider.request_count() > 0);

    let body = provider.continuation_body();
    assert_eq!(body["version"], 1);
    assert_eq!(body["authority"], "none");
    assert_eq!(body["readiness"], readiness);
    assert!(
        body["pending_tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["agent_id"] == "pending-04" && task["state"] == "not_started")
    );
    assert!(
        body["accepted_decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|decision| decision["sequence"] == completion.sequence.get())
    );
    assert!(
        body["accepted_decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|decision| decision["sequence"] == later_accepted.get())
    );
    assert_eq!(body["omitted"]["unaccepted_decisions"], json!(unaccepted));
    // Unaccepted results replay by their stable identity, so the resumed
    // root can read each one with `coding_result`; replay carries no answer
    // content and no authority.
    let replayed = body["unaccepted_findings"].as_array().unwrap();
    assert_eq!(
        replayed
            .iter()
            .map(|finding| finding["sequence"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        unaccepted_sequences
            .iter()
            .copied()
            .take(32)
            .collect::<Vec<_>>(),
        "replay lists the oldest open results first, bounded to 32"
    );
    assert!(replayed.iter().all(|finding| {
        finding["agent_id"].is_string()
            && finding["outcome"].is_string()
            && finding.get("answer").is_none()
            && finding.get("content").is_none()
    }));
    assert_eq!(
        body["omitted"]["unaccepted_findings"],
        json!(unaccepted.saturating_sub(32))
    );
    assert!(body.to_string().len() <= 48 * 1024);
    assert!(body["omitted"]["task_history_bytes"].as_u64().unwrap() > 48 * 1024);
}

#[tokio::test]
async fn malformed_task_history_refuses_resume_proposal_before_provider_execution() {
    let provider = Provider::new(false);
    let fixture = Fixture::with_endpoint("continuation-malformed", Some(&provider.endpoint)).await;
    let request = direct_request(
        "continuation-malformed",
        "Make native child changes",
        vec!["new-a.txt".into(), "new-b.txt".into()],
    );
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    job.run(&NoopEventSink).await.unwrap();
    std::fs::write(
        fixture
            .root
            .join("coding")
            .join(job.job_id().as_str())
            .join("tasks.jsonl"),
        b"not-json\n",
    )
    .unwrap();
    let calls = provider.calls.load(Ordering::Acquire);
    assert!(
        fixture
            .application
            .propose_resume(job.job_id())
            .await
            .is_err()
    );
    assert_eq!(provider.calls.load(Ordering::Acquire), calls);
}

#[tokio::test]
async fn duplicate_legacy_assignments_fail_closed_on_the_readers_that_consume_them() {
    let provider = Provider::new(false);
    let fixture = Fixture::with_endpoint("duplicate-assembly", Some(&provider.endpoint)).await;
    let request = direct_request(
        "duplicate-assembly",
        "Use only one authoritative assignment history",
        vec!["new-a.txt".into(), "new-b.txt".into()],
    );
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    job.run(&NoopEventSink).await.unwrap();
    let history = fixture
        .root
        .join("coding")
        .join(job.job_id().as_str())
        .join("tasks.jsonl");
    let first = fs::read_to_string(&history).unwrap();
    let duplicate = first.lines().next().unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(&history)
        .unwrap()
        .write_all(format!("{duplicate}\n").as_bytes())
        .unwrap();

    assert!(matches!(
        job.status().await,
        Err(CodingError::InvalidRequest)
    ));
    // Direct-mode assembly sources its one candidate from the root working
    // checkout W and never reads the assignment history, so the duplicate
    // cannot reach it. Every owner surface that does consume the history still
    // fails closed: the advisory status above, and delegation admission here.
    // The delegated batch stays inside the approved ceiling, so the ambiguous
    // history is the only possible reason for the refusal.
    assert!(matches!(
        job.delegate(vec![CodingTask {
            agent_id: AgentId::parse("child-extra").unwrap(),
            subagent_type: "general".into(),
            task: "write for child-extra".into(),
            write_paths: vec!["new-a.txt".into()],
            dependencies: vec![],
        }])
        .await,
        Err(CodingError::InvalidRequest)
    ));
    let candidate = ManagedCheckoutId::parse("duplicate-assembly-candidate").unwrap();
    assert!(
        fixture
            .store
            .list_managed_checkouts(job.job_id(), None, 10)
            .await
            .unwrap()
            .items
            .iter()
            .all(|checkout| checkout.ownership.managed_checkout_id() != &candidate),
        "the refused delegation assembled no candidate"
    );
    assert_eq!(
        fixture
            .store
            .list_coding_children(job.job_id(), None, 10)
            .await
            .unwrap()
            .items
            .len(),
        2,
        "the refused delegation registered no child"
    );
}
