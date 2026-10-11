#[path = "continuation_support.rs"]
mod continuation_support;

use super::{provider::Provider, support::Fixture};
use continuation_support::CaptureProvider;
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingError, CodingJobRequest};
use saya_store::NewCodingChild;
use saya_types::{
    AgentId, CodingEventKind, CodingEventV1, CodingResultOutcome, InvocationId, JobId, JobState,
    JoinedResultV1, ManagedCheckoutId,
};
use serde_json::json;
use std::num::NonZeroU64;
use std::{io::Write, sync::atomic::Ordering};

#[tokio::test]
async fn continuation_context_compacts_large_history_without_losing_durable_identities() {
    let provider = CaptureProvider::new();
    let fixture = Fixture::with_endpoint("continuation-context", Some(&provider.endpoint)).await;
    let request = CodingJobRequest::new(
        JobId::parse("continuation-context").unwrap(),
        "Make dependent native child changes".into(),
        vec![
            "shared.txt".into(),
            "rename-before.txt".into(),
            "rename-after.txt".into(),
        ],
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
    job.run(&NoopEventSink).await.unwrap();
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

    let approval = CodingAuthorization::scoped(
        &fixture
            .application
            .propose_resume(job.job_id())
            .await
            .unwrap(),
    )
    .unwrap();
    let resumed = fixture
        .application
        .resume(job.job_id(), &approval, CancellationToken::new())
        .await
        .unwrap();
    let candidate = ManagedCheckoutId::parse("continuation-conflict").unwrap();
    assert!(matches!(
        resumed.review(&candidate).await,
        Err(CodingError::ConflictRetained { .. })
    ));
    let conflict = fixture
        .application
        .conflicts(resumed.job_id(), &candidate)
        .await
        .unwrap();
    let output = resumed.run(&NoopEventSink).await.unwrap();
    assert!(!output.truncated);
    assert!(provider.request_count() > 0);

    let body = provider.continuation_body();
    assert_eq!(body["version"], 1);
    assert_eq!(body["authority"], "none");
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
    assert_eq!(
        body["pending_conflicts"][0]["record_sha256"],
        conflict["provenance"]["record_sha256"]
    );
    assert!(body.to_string().len() <= 48 * 1024);
    assert!(body["omitted"]["task_history_bytes"].as_u64().unwrap() > 48 * 1024);
}

#[tokio::test]
async fn malformed_task_history_refuses_before_a_resumed_provider_request() {
    let provider = Provider::new(false);
    let fixture = Fixture::with_endpoint("continuation-malformed", Some(&provider.endpoint)).await;
    let request = CodingJobRequest::new(
        JobId::parse("continuation-malformed").unwrap(),
        "Make native child changes".into(),
        vec!["new-a.txt".into(), "new-b.txt".into()],
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
    let approval = CodingAuthorization::scoped(
        &fixture
            .application
            .propose_resume(job.job_id())
            .await
            .unwrap(),
    )
    .unwrap();
    let resumed = fixture
        .application
        .resume(job.job_id(), &approval, CancellationToken::new())
        .await
        .unwrap();
    let calls = provider.calls.load(Ordering::Acquire);
    assert!(resumed.run(&NoopEventSink).await.is_err());
    assert_eq!(provider.calls.load(Ordering::Acquire), calls);
}

fn append_large_pending_tasks(fixture: &Fixture, job: &JobId) {
    let tasks = (0..5)
        .map(|index| {
            json!({
                "agent_id": format!("pending-{index:02}"),
                "task": "x".repeat(16 * 1024),
                "write_paths": [],
                "dependencies": []
            })
        })
        .collect::<Vec<_>>();
    let path = fixture
        .root
        .join("coding")
        .join(job.as_str())
        .join("tasks.jsonl");
    let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    writeln!(file, "{}", serde_json::to_string(&tasks).unwrap()).unwrap();
}

async fn append_completion_history(fixture: &Fixture, job: &JobId) -> NonZeroU64 {
    let events = fixture
        .store
        .list_coding_events(job, None, 100)
        .await
        .unwrap();
    let mut sequence = events.events.last().unwrap().sequence().get();
    for index in 0..34 {
        sequence += 1;
        let agent = AgentId::parse(&format!("history-{index:02}")).unwrap();
        let invocation = InvocationId::parse(&format!("inv-history-{index:02}")).unwrap();
        fixture
            .store
            .create_coding_child(NewCodingChild {
                job_id: job.clone(),
                agent_id: agent.clone(),
                current_invocation_id: invocation.clone(),
            })
            .await
            .unwrap();
        let running = CodingEventV1::new(
            NonZeroU64::new(sequence).unwrap(),
            job.clone(),
            Some(agent.clone()),
            Some(invocation.clone()),
            CodingEventKind::StateChanged {
                state: JobState::Running,
            },
        )
        .unwrap();
        fixture.store.append_coding_event(running).await.unwrap();
        sequence += 1;
        let sequence = NonZeroU64::new(sequence).unwrap();
        let result = JoinedResultV1::new(
            job.clone(),
            agent.clone(),
            invocation.clone(),
            CodingResultOutcome::Completed,
            format!("durable decision {index}"),
            false,
        )
        .unwrap();
        let event = CodingEventV1::new(
            sequence,
            job.clone(),
            Some(agent),
            Some(invocation),
            CodingEventKind::StateChanged {
                state: JobState::Completed,
            },
        )
        .unwrap();
        fixture
            .store
            .record_coding_completion(event, &result)
            .await
            .unwrap();
    }
    NonZeroU64::new(sequence).unwrap()
}
