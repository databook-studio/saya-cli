//! What a resume decides before it asks: a job that is not clear for resume is
//! refused with nothing written and no question put to the human, and the
//! verdict is re-read after consent because the job can change while the human
//! answers.

#![cfg(unix)]

use super::super::super::{CodingApplication, CodingError};
use super::super::parent_turn_fixture::{Fixture, in_place};
use super::super::parent_turn_provider::Scripted;
use super::{RUN, until};
use saya_agent::{ApprovalDecider, CancellationToken, NoopEventSink, ToolDefinition};
use saya_store::{NewCodingChild, SqliteStateStore};
use saya_types::{AgentId, CodingEventKind, CodingEventV1, InvocationId, JobId, JobState};
use serde_json::Value;
use std::{
    num::NonZeroU64,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

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

/// A decider that counts the questions it is asked and always answers yes.
struct Counting(Arc<AtomicUsize>);

#[async_trait::async_trait]
impl ApprovalDecider for Counting {
    async fn approve(&self, _tool: &ToolDefinition, _arguments: &Value) -> bool {
        self.0.fetch_add(1, Ordering::AcqRel);
        true
    }
}

/// A resume of a job whose root turn is still running decides before it asks:
/// the human is not called, no event is appended and no approval row is
/// written, so the running turn keeps the sequences it is about to use.
#[tokio::test]
async fn a_resume_of_a_running_job_asks_nothing_and_writes_nothing() {
    let provider = Scripted::start();
    provider.hold_first();
    let fixture = in_place("resume-silent", &provider.endpoint).await;
    let id = fixture.job.job_id().clone();
    let application = Arc::clone(&fixture.job.application);
    let running = {
        let job = Arc::clone(&fixture.job);
        tokio::spawn(async move { job.run(&NoopEventSink).await })
    };
    until(|| provider.arrived() >= 1).await;

    let before = tail(&application, &id).await;
    let audited_before = audited(&application, &id).await;
    let asked = Arc::new(AtomicUsize::new(0));
    let refused = application
        .resume(&id, &Counting(Arc::clone(&asked)), CancellationToken::new())
        .await;
    let Err(error) = refused else {
        panic!("a running job must refuse the resume");
    };
    assert!(
        matches!(error, CodingError::NotResumable { .. }),
        "a running job refuses the resume: {error:?}"
    );
    assert_eq!(
        asked.load(Ordering::Acquire),
        0,
        "the refusal asks the human nothing"
    );
    assert_eq!(
        tail(&application, &id).await,
        before,
        "the refusal appends no event to the running job's stream"
    );
    assert_eq!(
        audited(&application, &id).await,
        audited_before,
        "and writes no approval audit row"
    );

    provider.release();
    let held = tokio::time::timeout(RUN, running)
        .await
        .expect("the held turn must settle")
        .expect("the held turn's task must not panic");
    assert!(held.is_ok(), "the held run must still finish: {held:?}");
}

/// A decider that starts a helper on the job while it holds the question, so
/// the job is no longer clear for resume by the time consent arrives.
struct AHelperStarts {
    store: SqliteStateStore,
    job: JobId,
    asked: AtomicUsize,
}

#[async_trait::async_trait]
impl ApprovalDecider for AHelperStarts {
    async fn approve(&self, _tool: &ToolDefinition, _arguments: &Value) -> bool {
        self.asked.fetch_add(1, Ordering::AcqRel);
        let child = self
            .store
            .create_coding_child(NewCodingChild {
                job_id: self.job.clone(),
                agent_id: AgentId::parse("agent-late").unwrap(),
                current_invocation_id: InvocationId::parse("invocation-late").unwrap(),
            })
            .await
            .expect("the helper is recorded");
        let sequence = self
            .store
            .coding_recovery_status(&self.job)
            .await
            .expect("the recovery status reads")
            .last_sequence
            + 1;
        self.store
            .append_coding_event(
                CodingEventV1::new(
                    NonZeroU64::new(sequence).unwrap(),
                    self.job.clone(),
                    Some(child.agent_id),
                    Some(child.current_invocation_id),
                    CodingEventKind::StateChanged {
                        state: JobState::Running,
                    },
                )
                .unwrap(),
            )
            .await
            .expect("the helper starts");
        true
    }
}

/// The verdict is read again after consent: a job that took an open fact while
/// the human answered is refused with the same typed refusal, and takes no
/// epoch.
#[tokio::test]
async fn a_job_that_takes_an_open_fact_while_the_human_answers_is_refused_after_consent() {
    let provider = Scripted::start();
    let Fixture {
        root: _,
        job,
        _root,
    } = in_place("resume-and-then", &provider.endpoint).await;
    let id = job.job_id().clone();
    let application = Arc::clone(&job.application);
    // One turn of an open run, ended between its turns: a job a resume may
    // still continue, and the state a live helper can be started in.
    let turn = tokio::time::timeout(RUN, job.run_open(&NoopEventSink))
        .await
        .expect("the turn must settle");
    assert!(turn.is_ok(), "the turn must run: {turn:?}");
    super::super::parent_turn_fixture::pause(&job).await;
    let epoch = super::resume_turns::epoch(&application, &id).await;
    // The open run's handle owns the folder lease; a resume takes it only once
    // that handle is gone.
    drop(job);

    let decider = AHelperStarts {
        store: application.store.clone(),
        job: id.clone(),
        asked: AtomicUsize::new(0),
    };
    let refused = application
        .resume(&id, &decider, CancellationToken::new())
        .await;
    assert_eq!(
        decider.asked.load(Ordering::Acquire),
        1,
        "the human was asked before the job changed"
    );
    let Err(error) = refused else {
        panic!("a job that is no longer clear must be refused after consent");
    };
    assert!(
        matches!(error, CodingError::NotResumable { .. }),
        "the re-read recovery facts refuse the resume: {error:?}"
    );
    assert!(
        error.to_string().contains("unresolved recovery fact"),
        "and the refusal says what is open: {error}"
    );
    assert_eq!(
        super::resume_turns::epoch(&application, &id).await,
        epoch,
        "the refused resume took no epoch"
    );
}
