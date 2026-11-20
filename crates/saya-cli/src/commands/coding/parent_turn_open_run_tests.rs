//! A resume never writes into a job a live run owns.
//!
//! An open run idle between its turns holds the folder lease with no recovery
//! fact open, so the recovery verdict alone clears the job; the ownership check
//! is what refuses the resume, and it does so before any question or append.
//! A lease a dead owner left behind is not a live owner, so today's recovery
//! behaviour stands.

#![cfg(unix)]

use super::super::super::{CodingApplication, CodingError, NativeCodingJob};
use super::super::parent_turn_fixture::{Fixture, in_place};
use super::super::parent_turn_provider::Scripted;
use super::{RUN, first_turn, next_turn};
use saya_agent::{ApprovalDecider, CancellationToken, NoopEventSink, ToolDefinition};
use saya_types::JobId;
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

/// A decider that counts the questions it is asked and always answers yes.
struct Counting(Arc<AtomicUsize>);

#[async_trait::async_trait]
impl ApprovalDecider for Counting {
    async fn approve(&self, _tool: &ToolDefinition, _arguments: &Value) -> bool {
        self.0.fetch_add(1, Ordering::AcqRel);
        true
    }
}

/// The job's next free event sequence, as its durable tail states it.
async fn tail(application: &Arc<CodingApplication>, id: &JobId) -> u64 {
    application
        .store
        .coding_recovery_status(id)
        .await
        .expect("the recovery status reads")
        .last_sequence
}

/// How many approval audit rows the job holds.
async fn audited(application: &Arc<CodingApplication>, id: &JobId) -> usize {
    application
        .store
        .list_coding_approval_requests(id, None, 100)
        .await
        .expect("the job's approval audit reads")
        .items
        .len()
}

/// The root turn boundaries the run has recorded, in write order.
fn boundaries(job: &NativeCodingJob) -> Vec<Value> {
    super::records::records(job, "observations.jsonl")
        .into_iter()
        .filter(|record| record["phase"] == "turn_started")
        .collect()
}

/// The one folder lease file the state directory holds.
fn lease_file(application: &CodingApplication) -> PathBuf {
    let directory = application.state_root.join("inplace-leases");
    std::fs::read_dir(&directory)
        .expect("the lease folder exists")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "lock")
        })
        .expect("the job holds one folder lease file")
}

/// A resume of a job whose run is open and idle between its turns is refused by
/// the live owner before anything is asked or written, and the open run's next
/// turn still runs and records on the stream it owns.
#[tokio::test]
async fn a_resume_of_a_run_idle_between_turns_asks_and_writes_nothing() {
    let provider = Scripted::start();
    let fixture = in_place("resume-open-run", &provider.endpoint).await;
    let id = fixture.job.job_id().clone();
    let application = Arc::clone(&fixture.job.application);
    // One turn of an open run, ended normally: the run keeps its session and
    // its folder lease, and no recovery fact is open.
    let first = first_turn(&fixture.job).await;
    assert!(first.is_ok(), "the first turn must run: {first:?}");
    let before = tail(&application, &id).await;
    let audited_before = audited(&application, &id).await;
    let epoch_before = super::resume_turns::epoch(&application, &id).await;

    let asked = Arc::new(AtomicUsize::new(0));
    let refused = application
        .resume(&id, &Counting(Arc::clone(&asked)), CancellationToken::new())
        .await;
    let Err(error) = refused else {
        panic!("an open run must refuse the resume of its own job");
    };
    assert!(
        matches!(error, CodingError::InPlaceRootBusy),
        "the live owner refuses the resume: {error:?}"
    );
    assert_eq!(
        asked.load(Ordering::Acquire),
        0,
        "the refusal asks the human nothing"
    );
    assert_eq!(
        tail(&application, &id).await,
        before,
        "the refusal appends no event to the open run's stream"
    );
    assert_eq!(
        audited(&application, &id).await,
        audited_before,
        "and writes no approval audit row"
    );
    assert_eq!(
        super::resume_turns::epoch(&application, &id).await,
        epoch_before,
        "and takes no epoch"
    );

    // The refusal left the open run its stream and its ledger: the next turn
    // runs and records as it does with no resume attempting anything.
    let second = next_turn(&fixture.job, "second message").await;
    assert!(
        second.is_ok(),
        "the open run's next turn must run: {second:?}"
    );
    let recorded = boundaries(&fixture.job);
    assert_eq!(
        recorded.len(),
        2,
        "each turn of the run records its own boundary: {recorded:?}"
    );
    let bodies = provider.bodies();
    assert_eq!(
        bodies.len(),
        2,
        "each turn sends exactly one provider request: {bodies:?}"
    );
    assert!(
        bodies[1].contains("second message"),
        "the second request carries the new message"
    );
}

/// A lease a dead owner left behind is not a live owner: the job resumes with
/// exactly one consent, and takes the lease the dead run released.
#[tokio::test]
async fn a_stale_lease_from_a_dead_owner_still_resumes_with_one_consent() {
    let provider = Scripted::start();
    let Fixture {
        root: _,
        job,
        _root,
    } = in_place("resume-stale-lease", &provider.endpoint).await;
    let id = job.job_id().clone();
    let application = Arc::clone(&job.application);
    let ran = tokio::time::timeout(RUN, job.run(&NoopEventSink))
        .await
        .expect("the run must settle");
    assert!(ran.is_ok(), "the run must finish: {ran:?}");
    let lease = lease_file(&application);
    let epoch_before = super::resume_turns::epoch(&application, &id).await;
    // The owner is gone: the operating system released its lock with its last
    // handle, and only the lock file stays behind.
    drop(job);
    assert!(lease.is_file(), "the dead owner's lease file stays on disk");

    let asked = Arc::new(AtomicUsize::new(0));
    let resumed = application
        .resume(&id, &Counting(Arc::clone(&asked)), CancellationToken::new())
        .await;
    let resumed = match resumed {
        Ok(job) => job,
        Err(error) => panic!("a stale lease must not refuse the resume: {error:?}"),
    };
    assert_eq!(
        asked.load(Ordering::Acquire),
        1,
        "the resume asks exactly one consent"
    );
    assert_eq!(
        super::resume_turns::epoch(&application, &id).await,
        epoch_before + 1,
        "and starts the run's next epoch"
    );
    drop(resumed);
}
