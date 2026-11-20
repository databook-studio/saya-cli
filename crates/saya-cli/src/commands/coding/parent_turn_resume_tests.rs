//! A detached run keeps resume (phase-3 unit 3-D): one job is continued by the
//! same `saya job resume` whether its run is over or was never split into more
//! than one turn, and a job whose root turn is still running refuses the resume
//! without disturbing the run it would have taken over.

#![cfg(unix)]

use super::super::super::{CodingApplication, CodingError};
use super::super::parent_turn_fixture::{Fixture, in_place};
use super::super::parent_turn_provider::Scripted;
use super::{RUN, until};
use crate::cli::CodingCommand;
use saya_agent::NoopEventSink;
use saya_types::JobId;
use serde_json::Value;
use std::sync::Arc;

/// `saya job resume <id> --approve`, at the surface the verb runs.
async fn resume(application: &Arc<CodingApplication>, id: &JobId) -> Result<Value, CodingError> {
    crate::commands::coding::dispatch_coding(
        application,
        CodingCommand::Resume {
            id: id.as_str().to_owned(),
            approve: true,
        },
    )
    .await
}

/// The job's runtime epoch, as its authority records it.
async fn epoch(application: &Arc<CodingApplication>, id: &JobId) -> u64 {
    application
        .store
        .coding_recovery_status(id)
        .await
        .expect("the recovery status reads")
        .authority
        .expect("the job keeps its runtime authority")
        .epoch
}

/// A finished job is still continued by `saya job resume`: the run ends
/// `completed`, the resume starts a new epoch over the same folder and answers,
/// and the run it starts ends completed too. The split into one turn per run
/// changed nothing about the between-process verb.
#[tokio::test]
async fn a_finished_job_is_still_continued_by_a_resume() {
    let provider = Scripted::start();
    let Fixture {
        root: _,
        job,
        _root,
    } = in_place("resume-finished", &provider.endpoint).await;
    let id = job.job_id().clone();
    let application = Arc::clone(&job.application);

    let run = tokio::time::timeout(RUN, job.run(&NoopEventSink))
        .await
        .expect("the single run must settle");
    assert!(run.is_ok(), "the single run must finish: {run:?}");
    assert_eq!(
        application.status(&id).await.expect("the status reads")["state"],
        "completed"
    );
    assert_eq!(epoch(&application, &id).await, 1);
    // The live handle owns the folder lease; a resume takes it only once the
    // ended run's handle is gone.
    drop(job);

    let result = resume(&application, &id)
        .await
        .expect("a finished job resumes");
    assert_eq!(result["job_id"], id.as_str(), "{result}");
    assert_eq!(result["exit_code"], 0, "{result}");
    assert_eq!(
        epoch(&application, &id).await,
        2,
        "the resume starts a new epoch"
    );
    assert_eq!(
        application.status(&id).await.expect("the status reads")["state"],
        "completed",
        "the resumed run answered and finished"
    );
}

/// A job whose root turn is still running refuses `saya job resume`: the run's
/// own open recovery facts are the refusal, and the attempt takes no epoch, no
/// tool call and no model request from the run it could not take over.
#[tokio::test]
async fn a_resume_while_the_root_turn_runs_is_refused_without_taking_the_epoch() {
    let provider = Scripted::start();
    provider.hold_first();
    let fixture = in_place("resume-running", &provider.endpoint).await;
    let id = fixture.job.job_id().clone();
    let application = Arc::clone(&fixture.job.application);
    let before = epoch(&application, &id).await;

    let running = {
        let job = Arc::clone(&fixture.job);
        tokio::spawn(async move { job.run(&NoopEventSink).await })
    };
    until(|| provider.arrived() >= 1).await;

    let charged = fixture.job.ledger.snapshot().await;
    let refused = resume(&application, &id).await;
    let Err(error) = refused else {
        panic!("a resume must be refused while the root turn runs: {refused:?}");
    };
    assert!(
        matches!(error, CodingError::NotResumable { .. }),
        "the running job's own open recovery facts refuse the resume: {error:?}"
    );
    assert!(
        error.to_string().contains("unresolved recovery facts"),
        "and the refusal says what is open: {error}"
    );
    assert_eq!(
        epoch(&application, &id).await,
        before,
        "the refused resume took no epoch"
    );
    assert!(
        fixture.job._in_place_lease.is_some(),
        "and the running job still holds its one lease"
    );
    let after = fixture.job.ledger.snapshot().await;
    assert_eq!(
        after.logical_requests, charged.logical_requests,
        "the refusal charged no model request to the run"
    );
    assert_eq!(after.tool_calls, charged.tool_calls, "and no tool call");

    // The attempt writes its approval audit into the job's own durable stream
    // before the recovery facts refuse it, so the held turn cannot settle
    // cleanly afterwards; its outcome is not asserted here. Release it, so the
    // fixture's scripted provider does not wait out its hold.
    provider.release();
    let _ = tokio::time::timeout(RUN, running).await;
}
