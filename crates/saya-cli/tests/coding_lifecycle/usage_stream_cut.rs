//! The observed failure, end to end: a child's model stream is cut after it
//! had already delivered a long reasoning flow. The loop re-sent the
//! byte-identical request until the retry allowance ran out (~21 minutes of
//! ~302 s cuts); the identical request is never re-sent. One cut, then a
//! continuation that carries the closed note. A time-cut does not lower later
//! output ceilings, including not to the 1,024 floor a visible scrap
//! impose.
use super::{provider::Provider, support::Fixture};
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingApplication, CodingAuthorization, CodingJobRequest};
use saya_types::{CodingResultOutcome, JobId};
use serde_json::Value;
use std::time::Duration;

/// The child's request bodies, in order, as the wire carried them.
fn child_bodies(provider: &Provider) -> Vec<Value> {
    provider
        .bodies
        .lock()
        .unwrap()
        .iter()
        .filter(|body| body.contains("UNTRUSTED CHILD TASK"))
        .map(|body| serde_json::from_str(body).unwrap())
        .collect()
}

#[tokio::test]
async fn a_cut_after_content_is_continued_not_resent_identically() {
    let provider = Provider::child_stream_cut_after_reasoning();
    let mut fixture = Fixture::with_endpoint("usage-stream-cut", Some(&provider.endpoint)).await;
    // The fixture's default schedule is empty ("do not retry"), which would
    // hide the observed failure: the identical request was
    // re-sent once per scheduled retry, so the eight-entry default
    // sent it four times.
    fixture.runtime.resolved.ai.retry_delays_ms = vec![5, 5, 5];
    // The configured output ceiling the later turns must keep. It is explicit
    // so the assertion below pins "unchanged", not "unset".
    fixture.runtime.resolved.ai.max_output_tokens = Some(4096);
    fixture.application = CodingApplication::open(
        fixture.runtime.clone(),
        fixture.store.clone(),
        &fixture.repository,
        &fixture.root.join("coding"),
        &fixture.git,
        &fixture.program_dir,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let request = CodingJobRequest::new(
        JobId::parse("usage-stream-cut").unwrap(),
        "Write the owned file with helpers".into(),
        vec!["tracked.txt".into()],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    let run = tokio::time::timeout(Duration::from_secs(60), job.run(&NoopEventSink))
        .await
        .unwrap();

    let bodies = child_bodies(&provider);
    let first = &bodies[0];
    assert_eq!(
        bodies.iter().filter(|body| *body == first).count(),
        1,
        "the identical request is never re-sent after a cut that delivered content"
    );
    assert_eq!(
        bodies.len(),
        3,
        "one cut, the continuation that carries the child past it, and the \
         child's own closing turn: {bodies:#?}"
    );

    // The continuation carries the closed note — bounded, naming the window,
    // and never a replay of the reasoning the cut had delivered.
    let messages = bodies[1]["messages"].as_array().unwrap();
    let note = messages.last().expect("a note was pushed");
    assert_eq!(note["role"], "user", "{note}");
    let note = note["content"].as_str().unwrap();
    assert!(
        note.contains("cut off by the request time limit after 0s"),
        "the note names the window the provider allowed: {note}"
    );
    assert!(
        !note.contains("one tool call") && !note.contains("smaller steps"),
        "a durable one-call order outlives compaction: {note}"
    );
    assert!(
        !serde_json::to_string(&bodies[1]).unwrap().contains("xxxx"),
        "the cut attempt's reasoning never re-enters request history"
    );

    // A time-cut is a clock. Later child turns keep the configured ceiling
    // and must not drop to 1,024.
    assert!(
        bodies
            .iter()
            .all(|body| body["max_completion_tokens"] == 4096),
        "a time-cut must not floor later turns to 1024: {bodies:#?}"
    );

    run.expect("the child continues past the cut and the job finishes");
    let completions = fixture
        .store
        .list_coding_completions(job.job_id(), None, 8)
        .await
        .unwrap();
    assert_eq!(completions.items.len(), 1, "{completions:?}");
    assert_eq!(
        completions.items[0].result.outcome(),
        CodingResultOutcome::Completed,
        "the child finished on the continuation"
    );
}
