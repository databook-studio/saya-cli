//! A job that meets one transient transport failure retries it, on the same
//! schedule, instead of settling `not_retried`.
//!
//! The evidence this reproduces: a real job whose provider connection died
//! before a response ended `provider_recovery / not_retried / provider_failure`
//! on attempt 1 with a nine-attempt limit, so a transient upstream fault cost
//! the whole run.

use super::{RUN_CEILING, fixture::Fixture, provider::Provider, script::Fault};
use saya_agent::{AgentEvent, AgentEventSink, ProviderRecoveryPhase, ProviderRecoveryReason};
use std::sync::{Arc, Mutex};

/// Two configured retries: enough to prove the failure is re-sent, short
/// enough that the test never waits on the schedule.
const RETRY_MS: &[u64] = &[10, 10];

/// The text the scripted provider answers with once the request is retried.
const RECOVERED: &str = "Recovered: the retried request answered";

#[derive(Default)]
struct RecordingSink {
    events: Mutex<Vec<AgentEvent>>,
}

#[async_trait::async_trait]
impl AgentEventSink for RecordingSink {
    async fn emit(&self, event: AgentEvent) {
        self.events.lock().unwrap().push(event);
    }
}

/// Runs one scripted job whose first request meets `fault`, and returns
/// whether it completed, the requests it made, and every event the sink saw.
async fn run_job(fault: Fault) -> (bool, usize, Vec<AgentEvent>) {
    let provider = Provider::new(super::script::Scenario::Transient { fault });
    let fixture = Fixture::retrying(
        &provider.endpoint,
        256 * 1024,
        Some(8_000_000),
        "transient-root",
        RETRY_MS,
    )
    .await;
    let job = fixture.create().await;
    let sink = Arc::new(RecordingSink::default());
    let completed = tokio::time::timeout(RUN_CEILING, job.run(sink.as_ref()))
        .await
        .expect("the scripted job finishes on its own")
        .is_ok();
    let events = sink.events.lock().unwrap().clone();
    (completed, provider.bodies().len(), events)
}

/// The recovery the loop reported for the failed attempt.
fn recovery(events: &[AgentEvent]) -> Vec<(ProviderRecoveryPhase, ProviderRecoveryReason, u8)> {
    events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::ProviderRecovery {
                phase,
                reason,
                attempt,
                ..
            } => Some((*phase, *reason, *attempt)),
            _ => None,
        })
        .collect()
}

fn delivered_text(events: &[AgentEvent]) -> String {
    events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::AssistantText { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// One transient transport fault costs one retry, never the job.
fn assert_retried(completed: bool, requests: usize, events: &[AgentEvent]) {
    let seen = recovery(events);
    assert!(
        completed,
        "the job survives one transient transport failure: {seen:?}"
    );
    assert_eq!(requests, 2, "the failed request was re-sent once");
    assert!(
        seen.contains(&(
            ProviderRecoveryPhase::Retrying,
            ProviderRecoveryReason::ProviderFailure,
            2
        )),
        "the transient failure is retried on attempt 2: {seen:?}"
    );
    assert!(
        !seen.iter().any(
            |(phase, reason, _)| *phase == ProviderRecoveryPhase::NotRetried
                && *reason == ProviderRecoveryReason::ProviderFailure
        ),
        "a transient transport failure is never settled as an unretried provider failure: {seen:?}"
    );
    assert!(
        delivered_text(events).contains(RECOVERED),
        "the retried attempt's answer reached the sink"
    );
}

/// The connection goes away after the request body and before any response.
#[tokio::test]
async fn a_job_retries_a_connection_lost_before_the_response() {
    let (completed, requests, events) = run_job(Fault::Close).await;
    assert_retried(completed, requests, &events);
}

/// Headers and one SSE keep-alive chunk arrive, then the socket goes away.
#[tokio::test]
async fn a_job_retries_a_stream_that_dies_after_its_first_chunk() {
    let (completed, requests, events) = run_job(Fault::Truncate).await;
    assert_retried(completed, requests, &events);
}
