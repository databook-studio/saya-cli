//! A further turn reports to the sink its caller passes (phase-4 unit 4.0).
//!
//! `run_next_turn` used to report to `NoopEventSink` whatever the caller held;
//! an attached surface needs the turn's own events downstream, so the sink is a
//! parameter — and one with nothing downstream still works, which is what every
//! job surface passes today.

#![cfg(unix)]

use super::super::TurnInputs;
use super::super::parent_turn_fixture::in_place;
use super::super::parent_turn_provider::{Reply, Scripted};
use super::RUN;
use saya_agent::{AgentEvent, AgentEventSink, NoopEventSink};
use std::sync::{Arc, Mutex};

/// A sink that keeps every event its turn emitted, so the test can say which
/// tool calls the caller saw.
#[derive(Default)]
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
    /// The tool calls the turn requested and completed, in emit order.
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

/// Test 2 — the sink a caller passes to `run_next_turn` receives that turn's
/// `ToolRequested` and `ToolCompleted` events.
#[tokio::test]
async fn a_further_turn_reports_its_tool_events_to_the_callers_sink() {
    let provider = Scripted::start_with_script(vec![
        Reply::Text("the first turn answered"),
        Reply::Write {
            path: "second.txt",
            content: "second\n",
        },
        Reply::Text("the second turn answered"),
    ]);
    let fixture = in_place("turn-sink", &provider.endpoint).await;
    let first = tokio::time::timeout(
        RUN,
        fixture.job.run_open(&NoopEventSink, TurnInputs::default()),
    )
    .await
    .expect("the first turn must settle");
    assert!(first.is_ok(), "the first turn must run: {first:?}");

    let sink = Recording::default();
    let second = tokio::time::timeout(RUN, fixture.job.run_next_turn(&sink, "second message"))
        .await
        .expect("the second turn must settle");
    assert!(second.is_ok(), "the second turn must run: {second:?}");

    let names = sink.tool_names();
    assert!(
        names.contains(&"requested:write".to_owned()),
        "the caller's sink saw the turn's ToolRequested: {names:?}"
    );
    assert!(
        names.contains(&"completed:write".to_owned()),
        "and its ToolCompleted: {names:?}"
    );
}
