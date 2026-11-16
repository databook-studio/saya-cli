//! `saya job show`, `saya job export` and the `saya start` result report the
//! provider usage a job already recorded, including the cache and reasoning
//! breakdowns and the model wall time its spans measured.

use super::{
    provider::Provider,
    support::{EVENT_WAIT, Fixture},
};
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingCommand, CodingJobRequest, dispatch_coding};
use saya_types::JobId;

#[tokio::test]
async fn job_show_and_export_report_provider_usage_totals() {
    let provider = Provider::usage_cached();
    let fixture = Fixture::with_endpoint("usage-report", Some(&provider.endpoint)).await;
    let request = CodingJobRequest::new(
        JobId::parse("usage-report").unwrap(),
        "Read the tracked file".into(),
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
    tokio::time::timeout(EVENT_WAIT, job.run(&NoopEventSink))
        .await
        .expect("the scripted run terminates")
        .unwrap();

    let status = fixture.application.status(job.job_id()).await.unwrap();
    let usage = &status["usage"];
    assert_eq!(usage["source"], "durable_transport_spans");
    assert_eq!(usage["complete"], true, "{usage}");
    assert_eq!(usage["requests"], 2, "{usage}");
    assert_eq!(usage["input"], 38_000, "{usage}");
    assert_eq!(usage["cached_input"], 36_000, "{usage}");
    assert_eq!(usage["output"], 2_000, "{usage}");
    assert_eq!(usage["reasoning"], 1_000, "{usage}");
    assert_eq!(
        usage["cost_usd"], 0.5,
        "two calls at a quarter each: {usage}"
    );
    assert_eq!(
        usage["model_wall_time_ms"],
        wall_time_ms(&fixture, job.job_id()).await,
        "the totals carry the spans' own measured wall time: {usage}"
    );
    assert!(
        usage.get("incomplete").is_none(),
        "a job whose spans all reported tokens names no gap: {usage}"
    );

    let progress = &status["progress"];
    assert_eq!(progress["turns"], 2, "{progress}");
    assert_eq!(progress["last_progress_turn"], 1, "{progress}");
    assert_eq!(progress["progress_events"]["new_read"], 1, "{progress}");

    let exported = fixture
        .application
        .export(job.job_id(), None, None)
        .await
        .unwrap();
    assert_eq!(exported["usage"], *usage, "export repeats the same totals");
    assert_eq!(
        exported["status"]["progress"], *progress,
        "export repeats the recorded progress"
    );
}

/// The wall time the job's own durable spans measured, in milliseconds.
async fn wall_time_ms(fixture: &Fixture, job: &JobId) -> u64 {
    let mut wall_ns = 0u64;
    let attempts = fixture
        .store
        .list_coding_transport_attempts(job, None, 100)
        .await
        .unwrap();
    for item in &attempts.items {
        if let Some(span) = &item.span {
            let data = span.data();
            if let Some(start) = data.dispatch_started_ns {
                wall_ns = wall_ns.saturating_add(data.finished_ns.saturating_sub(start));
            }
        }
    }
    wall_ns / 1_000_000
}

#[tokio::test]
async fn a_span_without_reported_tokens_leaves_usage_incomplete_and_names_the_gap() {
    // Mode 60: the helper's first stream is cut after dispatch and the retry
    // succeeds. The cut attempt is a durable span with no reported tokens, so
    // it consumed an unknown amount; the totals stay honest and name the gap.
    let provider = Provider::child_stream_cut_once();
    let fixture = Fixture::with_endpoint("usage-incomplete", Some(&provider.endpoint)).await;
    let request = CodingJobRequest::new(
        JobId::parse("usage-incomplete").unwrap(),
        "Write the owned file with one helper".into(),
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
    tokio::time::timeout(EVENT_WAIT, job.run(&NoopEventSink))
        .await
        .expect("the scripted run terminates")
        .unwrap();

    let status = fixture.application.status(job.job_id()).await.unwrap();
    let usage = &status["usage"];
    assert_eq!(usage["complete"], false, "{usage}");
    assert_eq!(
        usage["incomplete"]["input"], 1,
        "exactly the cut span lacked input: {usage}"
    );
    assert_eq!(
        usage["incomplete"]["output"], 1,
        "exactly the cut span lacked output: {usage}"
    );
    // The two root spans reported the scripted 10 input / 5 output; the cut
    // helper span is named as the gap and never estimated into the totals.
    assert_eq!(usage["requests"], 2, "{usage}");
    assert_eq!(usage["input"], 20, "{usage}");
    assert_eq!(usage["output"], 10, "{usage}");
    assert_eq!(
        usage["model_wall_time_ms"],
        wall_time_ms(&fixture, job.job_id()).await,
        "the cut span's measured duration still counts: {usage}"
    );
}

#[tokio::test]
async fn saya_start_result_reports_the_same_usage_totals() {
    let provider = Provider::usage_cached();
    let fixture = Fixture::with_endpoint("usage-start", Some(&provider.endpoint)).await;
    let result = dispatch_coding(
        &fixture.application,
        CodingCommand::Start {
            id: "usage-start".into(),
            task: "Read the tracked file".into(),
            write_paths: vec!["tracked.txt".into()],
            database_profiles: Vec::new(),
            tools: Vec::new(),
            plan: false,
            approve: true,
            in_place: false,
            managed: false,
        },
    )
    .await
    .unwrap();
    let usage = &result["usage"];
    assert_eq!(usage["source"], "durable_transport_spans", "{result}");
    assert_eq!(usage["complete"], true, "{result}");
    assert_eq!(usage["requests"], 2, "{result}");
    assert_eq!(usage["input"], 38_000, "{result}");
    assert_eq!(usage["cached_input"], 36_000, "{result}");
    assert_eq!(usage["output"], 2_000, "{result}");
    assert_eq!(usage["reasoning"], 1_000, "{result}");
    assert_eq!(usage["cost_usd"], 0.5, "{result}");
    assert!(
        usage["model_wall_time_ms"].is_u64(),
        "the start result carries the spans' measured wall time: {result}"
    );
}
