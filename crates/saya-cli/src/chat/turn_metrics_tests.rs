//! A live chat turn records its question's metrics through the wrapped sink.
//!
//! The old turn body's metrics pin (`agent/runtime_metrics_tests.rs`) proved
//! that one *real* turn — not a hand-fed metric list — records the
//! provider-establishment and local-attempt phases, a `first_useful` instant,
//! into the question's own counters, and that the front's sink still receives
//! every event through the wrapper. This is the same sentence on the live
//! path: [`JobPort::with_metrics`] + [`ChatTurn`] wrap the front's sink in
//! [`MetricsSink`], so a scripted turn of a composed run must record the same
//! figures a surface reads out of `LocalMetrics`.
//!
//! The second question's counters are the front's boundary
//! (`front::line::signals::begin_question`): a question that begins starts
//! empty, measured from its own start.

#![cfg(unix)]

use std::sync::{Arc, Mutex};

use saya_agent::{AgentEvent, AgentEventSink, AgentMetricPhase};

use super::port::{Prepared, RunPort};
use super::{ChatOutcome, JobPort};
use crate::commands::coding::TurnInputs;
use crate::commands::coding::parent_turn_fixture::in_place_writing;
use crate::commands::coding::parent_turn_provider::{Reply, Scripted};
use crate::local_metrics::{LocalMetrics, LocalPhaseSnapshot, phase_index};

/// The front's own sink: every event the turn emitted, in order.
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
    fn answered(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .any(|event| matches!(event, AgentEvent::AssistantText { .. }))
    }
}

/// The phase cell a known phase was recorded into.
fn phase(snapshot: &LocalPhaseSnapshot, phase: AgentMetricPhase) -> u64 {
    snapshot.phases[phase_index(phase).expect("known phase indexed")].samples
}

#[tokio::test]
async fn a_live_chat_turn_records_its_metrics_through_the_wrapped_sink() {
    let provider = Scripted::start_with_script(vec![Reply::Text("here is the answer")]);
    let fixture = in_place_writing("chat-metrics", &provider.endpoint).await;
    let metrics = Arc::new(LocalMetrics::new());
    let port = JobPort::new(
        Arc::clone(&fixture.job),
        TurnInputs {
            policy: None,
            plugins: None,
            ..TurnInputs::default()
        },
    )
    .with_metrics(Arc::clone(&metrics));
    assert_eq!(
        port.prepare().await.expect("the run must open"),
        Prepared::default(),
        "the prepare reports the run ready"
    );

    let sink = Recording::default();
    let outcome = port
        .next_turn(Arc::new(sink.clone()), "first message".to_owned())
        .await;
    assert_eq!(
        outcome,
        Ok(ChatOutcome::Answered),
        "the turn answered: {outcome:?}"
    );

    let snapshot = metrics.snapshot();
    assert!(
        phase(&snapshot, AgentMetricPhase::ProviderEstablishment) >= 1,
        "provider establishment recorded: {snapshot:?}"
    );
    assert!(
        phase(&snapshot, AgentMetricPhase::LocalAttempt) >= 1,
        "local attempt recorded: {snapshot:?}"
    );
    assert!(
        snapshot.first_useful.is_some(),
        "first useful recorded: {snapshot:?}"
    );
    assert!(
        sink.answered(),
        "the answer text crossed the wrapper to the front's sink"
    );
}

#[tokio::test]
async fn the_next_question_records_into_a_fresh_snapshot() {
    let provider = Scripted::start_with_script(vec![
        Reply::Text("the first answer"),
        Reply::Text("the second answer"),
    ]);
    let fixture = in_place_writing("chat-metrics-reset", &provider.endpoint).await;
    let metrics = Arc::new(LocalMetrics::new());
    let port = JobPort::new(Arc::clone(&fixture.job), TurnInputs::default())
        .with_metrics(Arc::clone(&metrics));
    assert!(port.prepare().await.is_ok(), "the run must open");
    let first = port
        .next_turn(Arc::new(Recording::default()), "first message".to_owned())
        .await;
    assert_eq!(first, Ok(ChatOutcome::Answered), "{first:?}");
    assert!(
        metrics.snapshot().first_useful.is_some(),
        "the first question recorded: {:?}",
        metrics.snapshot()
    );

    // The front starts the next question at the boundary between them.
    metrics.begin_question();
    assert_eq!(
        metrics.snapshot(),
        LocalPhaseSnapshot::default(),
        "a question that begins starts with an empty snapshot"
    );

    let second = port
        .next_turn(Arc::new(Recording::default()), "second message".to_owned())
        .await;
    assert_eq!(second, Ok(ChatOutcome::Answered), "{second:?}");
    let snapshot = metrics.snapshot();
    assert!(
        phase(&snapshot, AgentMetricPhase::ProviderEstablishment) >= 1,
        "the second question records its own establishment: {snapshot:?}"
    );
}

/// The wrapper is what the live turn installs: a port with no counters records
/// nothing and still forwards the stream, so a surface that asked for no
/// metrics loses none of the turn.
#[tokio::test]
async fn a_turn_without_a_collector_still_forwards_the_stream() {
    let provider = Scripted::start_with_script(vec![Reply::Text("an answer")]);
    let fixture = in_place_writing("chat-metrics-off", &provider.endpoint).await;
    let port = JobPort::new(Arc::clone(&fixture.job), TurnInputs::default());
    assert!(port.prepare().await.is_ok(), "the run must open");
    let sink = Recording::default();
    let outcome = port
        .next_turn(Arc::new(sink.clone()), "first message".to_owned())
        .await;
    assert_eq!(outcome, Ok(ChatOutcome::Answered), "{outcome:?}");
    assert!(
        sink.answered(),
        "the front's sink saw the answer with no collector attached"
    );
}
