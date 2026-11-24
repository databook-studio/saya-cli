//! An owner read taken while the run is parked must never wait on the store.
//!
//! The store is one connection. A read an owner has in flight is held until
//! that owner is polled again, so a read a second surface takes while the owner
//! sits parked has nothing to acquire: it waits out the pool's acquisition
//! timeout, and the whole store is reported unavailable to that surface. The
//! run's durable cancellation poll is what reads the store while the run waits.
//!
//! This case parks the run at its final answer — the state a second surface
//! reads a status in — and samples many parked instants, because a park is one
//! instant and the run's own read is only in flight for part of them. Every
//! sample reads the job row, the read the owner status begins with, and the
//! status itself alongside it.

use super::provider::Provider;
use super::support::Fixture;
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingJobRequest, NativeCodingJob};
use saya_types::{JobId, JobState};
use std::sync::Arc;
use std::time::{Duration, Instant};

const RUN: Duration = Duration::from_secs(60);

/// How long one owner read may wait before it is a read waiting on a parked
/// owner's connection. A free store answers a job row in microseconds; the
/// pool's own acquisition timeout is five seconds, which is what turns one such
/// wait into an unavailable store for the surface that made it.
const READ_CEILING: Duration = Duration::from_millis(50);

/// How long the case samples parked instants. The scripted provider holds the
/// final answer, and the fixture's own request timeout ends that hold after five
/// seconds: the sampling stays inside the wait it is about.
const SAMPLE: Duration = Duration::from_secs(3);

/// The fewest parked instants a reading is worth anything at. A machine fast
/// enough to sample fewer has not sampled the wait.
const MIN_PARKS: usize = 100;

/// Every this-many parks, the whole owner status is read beside the job row.
const STATUS_EVERY: usize = 256;

/// Opens the job the parity provider scripts: no approved write path, so the
/// run parks at its final answer until `release()`.
async fn parked_job(fixture: &Fixture) -> Arc<NativeCodingJob> {
    let request = CodingJobRequest::new(
        JobId::parse("parked-reads").unwrap(),
        "Read the owner status while this run is parked".into(),
        Vec::new(),
    )
    .unwrap();
    let proposal = fixture.application.propose(&request).await.unwrap();
    let approval = CodingAuthorization::scoped(&proposal).unwrap();
    fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap()
}

#[tokio::test]
async fn an_owner_read_never_waits_on_a_parked_runs_store_connection() {
    let provider = Provider::parity();
    let fixture = Fixture::with_endpoint("parked-reads", Some(&provider.endpoint)).await;
    let job = parked_job(&fixture).await;
    let sink = NoopEventSink;
    let run = job.run(&sink);
    tokio::pin!(run);
    // The provider holds the final answer, so the run cannot finish: drive it
    // until that answer is the thing it waits for.
    tokio::select! {
        body = wait_for_body(&provider, "call-coding_status") => {
            assert!(body.contains("call-coding_status"));
        }
        output = &mut run => panic!("the run finished before it parked: {output:?}"),
    }
    let started = Instant::now();
    let mut parks = 0;
    while started.elapsed() < SAMPLE {
        // One poll of the owner, then the read: from here the owner is parked
        // exactly as a surface showing something else leaves it.
        let _ = tokio::time::timeout(Duration::ZERO, &mut run).await;
        let read_started = Instant::now();
        let read =
            tokio::time::timeout(READ_CEILING, fixture.store.get_coding_job(job.job_id())).await;
        assert!(
            read.is_ok(),
            "the owner's read at parked instant {parks} waited {:?} on the run's store connection",
            read_started.elapsed()
        );
        let record = read
            .expect("the read completed")
            .expect("the job row reads while the run is parked")
            .expect("the job row is in the store");
        assert_eq!(record.state, JobState::Running);
        if parks % STATUS_EVERY == 0 {
            let status = job
                .status()
                .await
                .expect("the owner status reads while the run is parked");
            assert_eq!(status["job_id"], "parked-reads");
            assert_eq!(status["state"], "running");
        }
        parks += 1;
    }
    assert!(
        parks >= MIN_PARKS,
        "the case sampled {parks} parked instants in {SAMPLE:?}"
    );
    provider.release();
    tokio::time::timeout(RUN, &mut run).await.unwrap().unwrap();
}

async fn wait_for_body(provider: &Provider, needle: &str) -> String {
    let deadline = tokio::time::Instant::now() + RUN;
    loop {
        let captured = provider
            .bodies
            .lock()
            .unwrap()
            .iter()
            .find(|body| body.contains(needle))
            .cloned();
        if let Some(body) = captured {
            return body;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the provider never captured a request containing {needle}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
