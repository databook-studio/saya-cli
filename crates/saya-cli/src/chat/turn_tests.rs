//! One chat turn is one `run_next_turn` on the job engine (phase-4 unit A4).
//!
//! A session states what its run is opened with, then sends messages: each is a
//! turn of that one run — same epoch, same lease — reported to the front's sink
//! with the metric names a surface already reads. Esc stops the turn in flight
//! and never the run, so the next message is a fresh turn. A turn's own budget
//! stop ends the turn and leaves the run open.

#![cfg(unix)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use saya_agent::{AgentEvent, AgentEventSink, BudgetDimension, NoopEventSink, TurnPolicy};

use super::JobPort;
use crate::chat::port::RunPort;
use crate::chat::{Budget, ChatOutcome};
use crate::commands::coding::TurnInputs;
use crate::commands::coding::parent_turn_fixture::in_place_writing;
use crate::commands::coding::parent_turn_provider::{Reply, Scripted};

/// Load ceiling, not a product bound: how long one scripted turn may take.
const RUN: Duration = Duration::from_secs(120);

/// A sink that keeps every event its turn emitted, so the test can say what the
/// front was shown — the front's own sink, passed through the port unchanged.
#[derive(Default, Clone)]
struct Recording(Arc<Mutex<Vec<AgentEvent>>>);

#[async_trait::async_trait]
impl AgentEventSink for Recording {
    async fn emit(&self, event: AgentEvent) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event);
    }
}

impl Recording {
    /// The tool calls the turns requested and completed, in emit order.
    fn tool_names(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter_map(|event| match event {
                AgentEvent::ToolRequested { name, .. } => Some(format!("requested:{name}")),
                AgentEvent::ToolCompleted { name, .. } => Some(format!("completed:{name}")),
                _ => None,
            })
            .collect()
    }
}

/// Waits until `settled` holds, bounded by the load ceiling.
async fn until(settled: impl Fn() -> bool) {
    let deadline = Instant::now() + RUN;
    while !settled() {
        assert!(Instant::now() < deadline, "the condition must settle");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Test 1 — prepare then two sends are two turns of one run, and the front's
/// sink saw each turn's tool steps.
#[tokio::test]
async fn prepare_then_two_sends_are_two_turns_of_one_run() {
    let provider = Scripted::start_with_script(vec![
        Reply::Write {
            path: "one.txt",
            content: "one\n",
        },
        Reply::Text("the first turn answered"),
        Reply::Text("the second turn answered"),
    ]);
    let fixture = in_place_writing("chat-two-sends", &provider.endpoint).await;
    let epoch = fixture.job.epoch();
    let sink = Recording::default();
    let port = JobPort::new(Arc::clone(&fixture.job), TurnInputs::default());

    let prepared = port.prepare().await;
    assert!(
        prepared.is_ok(),
        "the run must open without a turn: {prepared:?}"
    );
    assert!(
        provider.arrived() == 0,
        "the open runs no turn: {:?}",
        provider.bodies()
    );

    let first = port
        .next_turn(Arc::new(sink.clone()), "first message".to_owned())
        .await;
    assert_eq!(
        first,
        Ok(ChatOutcome::Answered),
        "the first message ran as a turn that answered"
    );
    let second = port
        .next_turn(Arc::new(sink.clone()), "second message".to_owned())
        .await;
    assert_eq!(
        second,
        Ok(ChatOutcome::Answered),
        "the second message ran as another turn of the run"
    );

    assert_eq!(
        fixture.job.epoch(),
        epoch,
        "both messages are turns of the one epoch"
    );
    assert!(
        fixture.job.holds_folder_lease(),
        "both messages ran on the one lease the run holds"
    );
    let tools = sink.tool_names();
    assert!(
        tools.contains(&"requested:write".to_owned()),
        "the front's sink saw the turn's ToolRequested: {tools:?}"
    );
    assert!(
        tools.contains(&"completed:write".to_owned()),
        "the front's sink saw the turn's ToolCompleted: {tools:?}"
    );
}

/// Test 2 — Esc cancels the turn in flight only: the outcome is `Cancelled`, the
/// run's own token is untouched, and the next message is a turn of the same run.
#[tokio::test]
async fn cancelling_a_turn_leaves_the_run_taking_the_next_message() {
    let provider = Scripted::start();
    provider.hold_first();
    let fixture = in_place_writing("chat-cancel", &provider.endpoint).await;
    let epoch = fixture.job.epoch();
    let port = Arc::new(JobPort::new(
        Arc::clone(&fixture.job),
        TurnInputs::default(),
    ));
    assert!(port.prepare().await.is_ok(), "the run must open");

    let running = {
        let port = Arc::clone(&port);
        tokio::spawn(async move {
            port.next_turn(Arc::new(NoopEventSink), "first message".to_owned())
                .await
        })
    };
    until(|| provider.arrived() >= 1).await;

    port.cancel_turn().await;
    let cancelled = tokio::time::timeout(RUN, running)
        .await
        .expect("the cancelled turn must settle")
        .expect("the turn's task must not panic");
    provider.release();
    assert_eq!(
        cancelled,
        Ok(ChatOutcome::Cancelled),
        "the operator cancelled the turn that was running"
    );
    assert_eq!(
        fixture.job.cancellation().reason(),
        None,
        "the cancel ended the turn and never the run"
    );
    assert!(
        fixture.job.epoch() == epoch && fixture.job.holds_folder_lease(),
        "the run is still open, on its own epoch and lease"
    );

    let second = tokio::time::timeout(
        RUN,
        port.next_turn(Arc::new(NoopEventSink), "second message".to_owned()),
    )
    .await
    .expect("the further turn must settle");
    assert_eq!(
        second,
        Ok(ChatOutcome::Answered),
        "the next message is admitted on the same epoch: {second:?}"
    );
    assert_eq!(
        fixture.job.epoch(),
        epoch,
        "the further turn is another turn of the same epoch"
    );
}

/// Test 3 — a turn the question budget stops reads as `Stopped(Budget)`, and the
/// run stays open for the next message.
#[tokio::test]
async fn a_turn_budget_stop_reads_as_stopped_and_leaves_the_run_open() {
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
    let fixture = in_place_writing("chat-turn-budget", &provider.endpoint).await;
    // The policy the session states when it opens the run — 3.4b's: one request
    // more than a single call and its answer, so a three-request turn stops.
    let inputs = TurnInputs {
        policy: Some(TurnPolicy {
            max_requests: Some(2),
            max_tool_calls: None,
            max_known_tokens: None,
            max_elapsed: None,
        }),
        plugins: None,
        // No history: this turn's budget is what the test reads.
        seed: Vec::new(),
    };
    let port = JobPort::new(Arc::clone(&fixture.job), inputs);
    assert!(
        port.prepare().await.is_ok(),
        "the run must open under the policy it was given"
    );

    let stopped = port
        .next_turn(Arc::new(NoopEventSink), "first message".to_owned())
        .await;
    assert_eq!(
        stopped,
        Ok(ChatOutcome::Stopped(Budget {
            dimension: BudgetDimension::LogicalRequests,
        })),
        "a spent question budget is the turn's stop, not a failure: {stopped:?}"
    );
    assert_eq!(
        provider.bodies().len(),
        2,
        "two requests were admitted and the third never arrived"
    );
    assert!(
        fixture.job.holds_folder_lease(),
        "the run that stopped its turn is still open"
    );

    let next = port
        .next_turn(Arc::new(NoopEventSink), "second message".to_owned())
        .await;
    assert_eq!(
        next,
        Ok(ChatOutcome::Answered),
        "the run keeps taking turns after a budget stop: {next:?}"
    );
}
