//! A schedule session holds one lease for its whole life, admits
//! children while it runs, starts each child only when the slot bound allows,
//! and gives every child its own completion signal. A child's own failure never
//! cancels its siblings; the owner, the caller and the deadline still stop the
//! whole session.

use super::{
    child_scheduler::{ledger, node},
    fixture::Fixture,
    openai_server::{ANSWER_RESPONSE, LocalOpenAi},
};
use saya_agent::{CancellationReason, CancellationToken};
use saya_harness::coding::{
    budget::JobBudgetLedger,
    coordinator::{
        ChildOutcome, ScheduleError, ScheduleLimits, ScheduleReport, ScheduleSession, ScheduleStop,
        ScheduledChild,
    },
};
use std::{sync::Arc, time::Duration};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

/// A session driven in its own task, with the submit side and caller token the
/// job would hold.
struct Running {
    session: ScheduleSession,
    caller: CancellationToken,
    task: JoinHandle<Result<ScheduleReport, ScheduleError>>,
    progress: JoinHandle<()>,
}

async fn open(
    first: Vec<ScheduledChild>,
    limits: ScheduleLimits,
    ledger: JobBudgetLedger,
) -> (Running, Vec<oneshot::Receiver<ChildOutcome>>) {
    let (sender, mut receiver) = mpsc::channel(128);
    let progress = tokio::spawn(async move { while receiver.recv().await.is_some() {} });
    let caller = CancellationToken::new();
    let (session, run, completions) =
        ScheduleSession::open(first, limits, &ledger, caller.clone(), sender)
            .await
            .expect("the first batch opens the session");
    let task = tokio::spawn(run.run());
    (
        Running {
            session,
            caller,
            task,
            progress,
        },
        completions,
    )
}

/// Closes the session and drains it: running children finish, then the report.
async fn finish(running: Running) -> ScheduleReport {
    let Running {
        session,
        task,
        progress,
        ..
    } = running;
    drop(session);
    let report = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("session must join")
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), progress)
        .await
        .expect("progress owner must finish")
        .unwrap();
    report
}

fn one_slot() -> ScheduleLimits {
    ScheduleLimits {
        max_active_children: 1,
        max_active_per_provider: 1,
    }
}

fn three_slots() -> ScheduleLimits {
    ScheduleLimits {
        max_active_children: 3,
        max_active_per_provider: 3,
    }
}

#[tokio::test]
async fn a_later_wave_of_children_runs_beside_the_first() {
    let fixture = Fixture::new("l1-two-waves").await;
    let ledger = ledger(&fixture).await;
    let mut a = LocalOpenAi::new().await;
    let mut b = LocalOpenAi::new().await;
    let first = node(&fixture, &a, "agent-a", vec![], "provider", Arc::default()).await;
    let (running, _first) = open(vec![first], three_slots(), ledger.clone()).await;
    // The first child is held inside its provider turn.
    assert_eq!(a.request().await, Some(1));
    let second = node(&fixture, &b, "agent-b", vec![], "provider", Arc::default()).await;
    let second_outcome = running.session.submit(second).await.unwrap();
    // The second child starts while the first is still running.
    assert_eq!(b.request().await, Some(1));
    a.respond_sse(ANSWER_RESPONSE);
    b.respond_sse(ANSWER_RESPONSE);
    let report = finish(running).await;
    assert!(report.stop.is_none());
    assert_eq!(report.outcomes.len(), 2);
    assert!(report.outcomes.iter().all(|outcome| outcome.result.is_ok()));
    assert!(report.not_started.is_empty());
    let second = second_outcome.await.unwrap();
    assert!(second.succeeded);
    assert_eq!(second.agent_id.as_str(), "agent-b");
    a.shutdown().await;
    b.shutdown().await;
    fixture.close().await;
}

#[tokio::test]
async fn a_child_over_the_bound_waits_for_a_slot_and_still_completes() {
    let fixture = Fixture::new("l1-bound-waits").await;
    let ledger = ledger(&fixture).await;
    let mut a = LocalOpenAi::new().await;
    let mut b = LocalOpenAi::new().await;
    let first = node(&fixture, &a, "agent-a", vec![], "provider", Arc::default()).await;
    let (running, _first) = open(vec![first], one_slot(), ledger.clone()).await;
    assert_eq!(a.request().await, Some(1));
    let second = node(&fixture, &b, "agent-b", vec![], "provider", Arc::default()).await;
    let second_outcome = running.session.submit(second).await.unwrap();
    // One slot is taken: the second child waits instead of being refused.
    assert_eq!(
        b.request_within(Duration::from_millis(300)).await,
        None,
        "a child over the bound must wait for a slot, not start"
    );
    a.respond_sse(ANSWER_RESPONSE);
    // The slot frees, and the waiting child starts.
    assert_eq!(b.request().await, Some(1));
    b.respond_sse(ANSWER_RESPONSE);
    let report = finish(running).await;
    assert!(report.stop.is_none());
    assert_eq!(report.outcomes.len(), 2);
    assert!(report.outcomes.iter().all(|outcome| outcome.result.is_ok()));
    assert!(second_outcome.await.unwrap().succeeded);
    a.shutdown().await;
    b.shutdown().await;
    fixture.close().await;
}

#[tokio::test]
async fn one_child_failing_leaves_its_sibling_running_to_its_own_outcome() {
    let fixture = Fixture::new("l1-child-isolation").await;
    let ledger = ledger(&fixture).await;
    let mut failing = LocalOpenAi::new().await;
    let mut sibling = LocalOpenAi::new().await;
    let first = node(
        &fixture,
        &failing,
        "agent-failing",
        vec![],
        "provider",
        Arc::default(),
    )
    .await;
    let (running, mut first_outcome) = open(vec![first], three_slots(), ledger.clone()).await;
    assert_eq!(failing.request().await, Some(1));
    let second = node(
        &fixture,
        &sibling,
        "agent-sibling",
        vec![],
        "provider",
        Arc::default(),
    )
    .await;
    let second_outcome = running.session.submit(second).await.unwrap();
    assert_eq!(sibling.request().await, Some(1));
    failing.respond_status(500);
    sibling.respond_sse(ANSWER_RESPONSE);
    let report = finish(running).await;
    // The failing child is recorded as failed; the session sets no batch-level
    // failure and the sibling still returns its own outcome.
    assert!(
        report.stop.is_none(),
        "one child's failure must not fail the batch: {:?}",
        report.stop
    );
    assert_eq!(report.outcomes.len(), 2);
    let failed = report
        .outcomes
        .iter()
        .find(|outcome| outcome.agent_id.as_str() == "agent-failing")
        .expect("the failing child is recorded");
    assert!(failed.result.is_err());
    let succeeded = report
        .outcomes
        .iter()
        .find(|outcome| outcome.agent_id.as_str() == "agent-sibling")
        .expect("the sibling is recorded");
    assert!(succeeded.result.is_ok());
    let failed = first_outcome.remove(0).await.unwrap();
    assert!(!failed.succeeded);
    assert!(failed.sequence.is_some(), "its own failure is durable");
    assert!(second_outcome.await.unwrap().succeeded);
    failing.shutdown().await;
    sibling.shutdown().await;
    fixture.close().await;
}

#[tokio::test]
async fn caller_cancellation_stops_every_child() {
    let fixture = Fixture::new("l1-caller-cancel").await;
    let ledger = ledger(&fixture).await;
    let mut a = LocalOpenAi::new().await;
    let mut b = LocalOpenAi::new().await;
    let first = node(&fixture, &a, "agent-a", vec![], "provider", Arc::default()).await;
    let (running, mut first_outcome) = open(vec![first], three_slots(), ledger.clone()).await;
    assert_eq!(a.request().await, Some(1));
    let second = node(&fixture, &b, "agent-b", vec![], "provider", Arc::default()).await;
    let second_outcome = running.session.submit(second).await.unwrap();
    assert_eq!(b.request().await, Some(1));
    running.caller.cancel_with_reason(CancellationReason::User);
    let report = finish(running).await;
    assert_eq!(
        report.stop,
        Some(ScheduleStop::Cancelled(CancellationReason::User))
    );
    assert_eq!(report.outcomes.len(), 2);
    assert!(
        report
            .outcomes
            .iter()
            .all(|outcome| outcome.result.is_err())
    );
    assert!(!first_outcome.remove(0).await.unwrap().succeeded);
    assert!(!second_outcome.await.unwrap().succeeded);
    a.shutdown().await;
    b.shutdown().await;
    fixture.close().await;
}

#[tokio::test]
async fn close_then_drain_ends_with_a_report_naming_every_child() {
    let fixture = Fixture::new("l1-close-drain").await;
    let ledger = ledger(&fixture).await;
    let mut a = LocalOpenAi::new().await;
    let mut b = LocalOpenAi::new().await;
    let first = node(&fixture, &a, "agent-a", vec![], "provider", Arc::default()).await;
    let (running, _first) = open(vec![first], three_slots(), ledger.clone()).await;
    assert_eq!(a.request().await, Some(1));
    let second = node(&fixture, &b, "agent-b", vec![], "provider", Arc::default()).await;
    let _second_outcome = running.session.submit(second).await.unwrap();
    assert_eq!(b.request().await, Some(1));
    // Close while both run: the report still names every child.
    a.respond_sse(ANSWER_RESPONSE);
    b.respond_sse(ANSWER_RESPONSE);
    let report = finish(running).await;
    assert!(report.stop.is_none());
    assert!(report.not_started.is_empty());
    let mut agents = report
        .outcomes
        .iter()
        .map(|outcome| outcome.agent_id.as_str().to_owned())
        .collect::<Vec<_>>();
    agents.sort();
    assert_eq!(agents, vec!["agent-a".to_owned(), "agent-b".to_owned()]);
    assert!(report.outcomes.iter().all(|outcome| outcome.result.is_ok()));
    a.shutdown().await;
    b.shutdown().await;
    fixture.close().await;
}
