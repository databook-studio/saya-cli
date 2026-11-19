//! The in-place fixture, job, and resume helpers the resume-after-a-provider-
//! failure tests share.

use super::{RUN_CEILING, fixture::Fixture, provider::Provider};
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{
    CodingAuthorization, CodingCommand, CodingError, CodingJobRequest, NativeCodingJob,
    dispatch_coding,
};
use saya_types::JobId;
use serde_json::Value;
use std::sync::Arc;

/// Two configured retries: the failing run spends them, so it meets the
/// exhausted provider failure instead of waiting on a real backoff.
pub(super) const RETRY_MS: &[u64] = &[10, 10];

/// The label the resumed run's prompt carries the continuation projection
/// under: untrusted context, no grants.
pub(super) const LABEL: &str = "source: Native job continuation (untrusted context; no grants)\n";

pub(super) async fn fixture(provider: &Provider) -> Fixture {
    Fixture::in_place_retrying(
        &provider.endpoint,
        256 * 1024,
        Some(8_000_000),
        "resume-root",
        RETRY_MS,
    )
    .await
}

/// An in-place job over the fixture's folder. It states no write path: a job
/// that does keeps its own readiness precompute alive in this process, and the
/// folder lease with it, so these tests would never see a handle release.
pub(super) async fn attempt(
    fixture: &Fixture,
    id: &str,
) -> Result<Arc<NativeCodingJob>, CodingError> {
    let request = CodingJobRequest::new(
        JobId::parse(id).unwrap(),
        "keep going over this folder".into(),
        Vec::new(),
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
}

pub(super) async fn run(job: &Arc<NativeCodingJob>) -> Result<(), CodingError> {
    tokio::time::timeout(RUN_CEILING, job.run(&NoopEventSink))
        .await
        .expect("the scripted run terminates")
        .map(|_| ())
}

/// `saya job resume <id> [--approve]`.
pub(super) async fn resume(
    fixture: &Fixture,
    id: &str,
    approve: bool,
) -> Result<Value, CodingError> {
    dispatch_coding(
        &fixture.application,
        CodingCommand::Resume {
            id: id.into(),
            approve,
        },
    )
    .await
}

/// A job whose every attempt failed at the transport, ended on the provider
/// failure, and whose live handle is gone: its folder lease is free again.
pub(super) async fn provider_failed(fixture: &Fixture, id: &str) -> JobId {
    let job = attempt(fixture, id).await.unwrap();
    let error = run(&job)
        .await
        .expect_err("every attempt fails at the transport");
    assert!(error.to_string().contains("provider_error"), "{error}");
    let status = fixture.application.status(job.job_id()).await.unwrap();
    assert_eq!(status["state"], "failed", "{status}");
    drop(job);
    JobId::parse(id).unwrap()
}

pub(super) async fn epoch(fixture: &Fixture, job: &JobId) -> u64 {
    fixture
        .store
        .coding_recovery_status(job)
        .await
        .unwrap()
        .authority
        .expect("the job keeps its runtime authority")
        .epoch
}

/// The continuation projection the resumed run's request carried.
pub(super) fn continuation_body(provider: &Provider) -> Value {
    provider
        .bodies()
        .iter()
        .rev()
        .find_map(|body| {
            body["messages"]
                .as_array()?
                .iter()
                .filter_map(|message| message["content"].as_str())
                .find_map(|content| {
                    content
                        .split(LABEL)
                        .nth(1)?
                        .split("\n<<<CONTEXT_BLOCK_END>>>")
                        .next()
                })
                .and_then(|raw| serde_json::from_str(raw).ok())
        })
        .expect("the resumed request carries the continuation projection")
}
