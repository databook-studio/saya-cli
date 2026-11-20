//! The DB question budget on a core turn (phase-3 unit 3.4b).
//!
//! The turn policy is an explicit, optional input of an open run: with one set,
//! a budget stop ends THAT turn with chat's partial wording and leaves the run
//! open, so the next turn admits requests again; with none — what every job turn
//! has today — a long turn is not capped by chat's 24 requests / 300 s.

#![cfg(unix)]

use super::super::super::NativeCodingJob;
use super::super::CodingError;
use super::super::parent_turn_fixture::in_place_writing;
use super::super::parent_turn_provider::{Reply, Scripted};
use saya_agent::{AgentOutput, NoopEventSink, TurnPolicy};
use std::time::Duration;

/// Load ceiling, not a product bound: how long one scripted turn may take.
const RUN: Duration = Duration::from_secs(120);

/// The chat's question budget stated by the test — no front end sets one yet —
/// narrow enough that one extra request trips it.
fn two_requests() -> TurnPolicy {
    TurnPolicy {
        max_requests: Some(2),
        max_tool_calls: None,
        max_known_tokens: None,
        max_elapsed: None,
    }
}

/// Runs one turn of the run: the first (`None`) or a further one, bounded so a
/// wedged turn fails the test instead of hanging it.
async fn turn(
    job: &std::sync::Arc<NativeCodingJob>,
    prompt: Option<&str>,
) -> Result<AgentOutput, CodingError> {
    let settled = match prompt {
        None => tokio::time::timeout(RUN, job.run_open(&NoopEventSink)).await,
        Some(prompt) => tokio::time::timeout(RUN, job.run_next_turn(prompt)).await,
    };
    settled.expect("the turn must settle")
}

/// Test 1 — a turn that spends its question budget stops with chat's wording,
/// the run stays open, and the next turn admits requests again on the same
/// epoch and the one lease.
#[tokio::test]
async fn a_turn_budget_stop_ends_the_turn_and_the_run_keeps_taking_turns() {
    let provider = Scripted::start_with_script(vec![
        Reply::Write {
            path: "one.txt",
            content: "one\n",
        },
        Reply::Write {
            path: "two.txt",
            content: "two\n",
        },
        Reply::Text("the second turn answered"),
    ]);
    let fixture = in_place_writing("turn-budget", &provider.endpoint).await;
    let epoch = fixture.job.epoch;
    fixture.job.set_turn_policy(Some(two_requests()));

    let stopped = turn(&fixture.job, None).await;
    assert!(
        stopped.is_ok(),
        "a spent question budget is a partial answer, not a run failure: {stopped:?}"
    );
    let stop = fixture
        .job
        .turn_stop()
        .expect("the turn states what stopped it");
    assert_eq!(
        stop.message(),
        "the run stopped on its logical-requests budget; any answer it produced is partial or absent",
        "chat's own budget wording, unchanged"
    );
    assert_eq!(
        provider.bodies().len(),
        2,
        "two requests were admitted and the third never arrived"
    );
    assert!(
        fixture.job._in_place_lease.is_some(),
        "the run is still open and holds its one folder lease"
    );

    let second = turn(&fixture.job, Some("second message")).await;
    assert!(
        second.is_ok(),
        "a further turn admits requests again: {second:?}"
    );
    assert_eq!(
        fixture.job.epoch, epoch,
        "a further turn is another turn of the same epoch"
    );
    assert_eq!(
        provider.bodies().len(),
        3,
        "the further turn reached the provider"
    );
    assert!(
        fixture.job._in_place_lease.is_some(),
        "the further turn ran on the lease the run already held"
    );
}

/// Test 2 — a job turn with no policy is not capped by the question budget: 30
/// progress-making tool calls, past the chat's 24 requests, complete with no
/// budget wording at all.
#[tokio::test]
async fn a_job_turn_with_no_policy_is_not_capped_by_the_question_budget() {
    let script = (0..30)
        .map(|_| Reply::DistinctWrite)
        .chain([Reply::Text("the long turn answered")])
        .collect();
    let provider = Scripted::start_with_script(script);
    let fixture = in_place_writing("turn-uncapped", &provider.endpoint).await;

    let output = turn(&fixture.job, None).await;
    let output = output.expect("a long turn of a job completes");
    assert_eq!(
        output.answer, "the long turn answered",
        "the turn ran to its own answer, not to a budget stop"
    );
    assert!(
        !output.truncated,
        "no budget stopped the turn: its answer is whole"
    );
    assert_eq!(
        provider.bodies().len(),
        31,
        "every request the turn needed was admitted"
    );
    assert_eq!(
        fixture.job.turn_stop(),
        None,
        "nothing stopped the turn, so there is no budget wording"
    );
}
