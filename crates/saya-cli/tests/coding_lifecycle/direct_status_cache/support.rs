//! The advisory-readiness-cache fixtures: a direct job that grants a program
//! set (so a command ATTEMPT reaches the runner instead of being refused at
//! approval), the scripted-run helpers, and the write into the working
//! checkout that no counter the cache key holds can see.

use crate::direct_task::root_checkout;
use crate::provider::Provider;
use crate::support::Fixture;
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingApplication, CodingAuthorization, CodingJobRequest, NativeCodingJob};
use saya_types::JobId;
use serde_json::Value;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// `coding_status` reads the "nothing changes" script makes, as
/// `script_direct_status` writes it.
pub(super) const READS: usize = 8;

/// Bound on one scripted run.
pub(super) const RUN: Duration = Duration::from_secs(60);

/// The bytes the tests write into W behind the cache's back: no tool call
/// makes them, so no counter the key holds moves.
pub(super) const BEHIND_THE_CACHE: &str = "written behind the cache\n";

/// Opens a direct-mode job over `write_paths`.
pub(super) async fn status_job(
    label: &str,
    mode: u8,
    write_paths: Vec<String>,
) -> (Fixture, Arc<NativeCodingJob>, Provider) {
    let provider = Provider::script(false, mode);
    let mut fixture = Fixture::with_endpoint(label, Some(&provider.endpoint)).await;
    fixture.runtime.resolved.limits.job.logical_requests = finite(400);
    // A status-cache scenario is repeated reads by construction; the job's
    // progress policy is off so the cache, not a guard, ends the run.
    fixture.runtime.resolved.limits.job.no_progress_turns = 0;
    fixture.runtime.resolved.limits.job.repeat_limit = 0;
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
        JobId::parse(label).unwrap(),
        "Direct-mode status reads with helpers".into(),
        write_paths,
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    (fixture, job, provider)
}

pub(super) async fn run(job: &Arc<NativeCodingJob>) {
    tokio::time::timeout(RUN, job.run(&NoopEventSink))
        .await
        .expect("the scripted run terminates")
        .unwrap();
}

/// The tagged status results the provider captured, in call order.
pub(super) fn status_reads(provider: &Provider) -> Vec<String> {
    let bodies = provider.bodies.lock().unwrap();
    let mut seen: Vec<String> = Vec::new();
    for body in bodies.iter() {
        let Ok(request) = serde_json::from_str::<Value>(body) else {
            continue;
        };
        for message in request["messages"].as_array().cloned().unwrap_or_default() {
            let Some(id) = message["tool_call_id"].as_str() else {
                continue;
            };
            if let Some(stage) = id.strip_prefix("call-status-")
                && !seen.iter().any(|read| read == stage)
            {
                seen.push(stage.to_owned());
            }
        }
    }
    seen
}

/// Waits until the provider has captured a request carrying `needle`.
async fn wait_for_body(provider: &Provider, needle: &str) {
    let deadline = Instant::now() + RUN;
    loop {
        if provider
            .bodies
            .lock()
            .unwrap()
            .iter()
            .any(|body| body.contains(needle))
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the provider never captured a request carrying {needle}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Runs a job whose script holds the call under test until the test has
/// written into W: the change lands on disk, and in NO counter.
pub(super) async fn run_behind_the_cache(
    fixture: &Fixture,
    job: &Arc<NativeCodingJob>,
    provider: &Provider,
) {
    let sink = NoopEventSink;
    let running = job.run(&sink);
    tokio::pin!(running);
    let ((), output) = tokio::join!(
        async {
            wait_for_body(provider, "call-status-0").await;
            std::fs::write(
                root_checkout(fixture, job).await.join("extra.txt"),
                BEHIND_THE_CACHE,
            )
            .unwrap();
            provider.release();
        },
        tokio::time::timeout(RUN, &mut running),
    );
    output.expect("the scripted run terminates").unwrap();
}

/// The generations the job's durable prepared receipts are bound to.
pub(super) fn prepared_generations(fixture: &Fixture, job: &NativeCodingJob) -> Vec<String> {
    let mut digests: Vec<String> =
        std::fs::read_dir(crate::direct_prepare::fixture::records_dir(fixture, job))
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter_map(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .strip_prefix("prepared-candidate-")
                            .and_then(|name| name.strip_suffix(".json"))
                            .map(str::to_owned)
                    })
                    .collect()
            })
            .unwrap_or_default();
    digests.sort();
    digests
}

pub(super) async fn resume(fixture: &Fixture, job: &JobId) -> Arc<NativeCodingJob> {
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose_resume(job).await.unwrap())
            .unwrap();
    fixture
        .application
        .resume(job, &approval, CancellationToken::new())
        .await
        .unwrap()
}

fn finite(value: u64) -> saya_config::BudgetLimit {
    saya_config::BudgetLimit::Finite(std::num::NonZeroU64::new(value).unwrap())
}
