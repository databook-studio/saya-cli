//! A plugin's system-prompt section rides the run's prompt, once.
//!
//! The prompt is composed once per run, not once per turn, so a section a
//! plugin states there reaches every turn of the run as the same bytes — which
//! is what a provider's prompt cache needs. An inactive plugin states nothing
//! at all: it contributes zero bytes to the request body.

#![cfg(unix)]

use super::super::parent_turn_fixture::in_place;
use super::super::parent_turn_provider::{Reply, Scripted};
use super::super::{RunPlugins, TurnInputs};
use super::RUN;
use saya_agent::{
    NoopEventSink, PluginRegistry, RunContext, RunPlugin, RunSurface, ToolDefinition, ToolExecutor,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

/// The section the active test plugin states, and the one an inactive plugin
/// would state were it asked.
const MARKER: &str = "toolbox-section-marker";
const SILENT: &str = "inactive-section-marker";

/// A sentence of the core prompt near its end: a section appended after the
/// core prompt follows it.
const CORE_TAIL: &str = "Never claim readiness or apply-back without owner receipts.";

/// A plugin whose only contribution is a system-prompt section.
///
/// The section names the call that produced it, so a run that composed the
/// prompt per turn would send turn 2 a different string than turn 1 — the
/// difference this file's first test exists to catch.
struct SectionPlugin {
    name: &'static str,
    active: bool,
    marker: &'static str,
    composed: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl RunPlugin for SectionPlugin {
    fn name(&self) -> &'static str {
        self.name
    }

    fn declared_tools(&self) -> &'static [&'static str] {
        &[]
    }

    fn activate(&self, _ctx: &RunContext) -> bool {
        self.active
    }

    fn definitions(&self, _ctx: &RunContext) -> Vec<ToolDefinition> {
        Vec::new()
    }

    fn executor(&self) -> Option<Arc<dyn ToolExecutor>> {
        None
    }

    fn system_section(&self, _ctx: &RunContext) -> Option<String> {
        let call = self.composed.fetch_add(1, Ordering::AcqRel) + 1;
        Some(format!("{} composed-call {call}", self.marker))
    }
}

/// The run context the test's plugins activate for.
fn context() -> RunContext {
    RunContext {
        surface: RunSurface::Chat,
        database_profiles: 0,
    }
}

/// The system message(s) of one request body, as the model receives them.
fn system_message(body: &str) -> String {
    let body: serde_json::Value = serde_json::from_str(body).expect("a request body is JSON");
    body["messages"]
        .as_array()
        .expect("a request carries its messages")
        .iter()
        .filter(|message| message["role"] == "system")
        .filter_map(|message| message["content"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Test 1 — the active plugin's section appears exactly once, after the core
/// prompt, and turn 2's system message is byte-identical to turn 1's.
#[tokio::test]
async fn a_plugins_section_rides_the_run_prompt_once_and_stays_put() {
    let provider = Scripted::start_with_script(vec![
        Reply::Text("the first turn answered"),
        Reply::Text("the second turn answered"),
    ]);
    let fixture = in_place("turn-section", &provider.endpoint).await;
    let composed = Arc::new(AtomicUsize::new(0));
    let registry = PluginRegistry::new(
        vec![Arc::new(SectionPlugin {
            name: "toolbox",
            active: true,
            marker: MARKER,
            composed: Arc::clone(&composed),
        })],
        &[],
    )
    .expect("the test plugin declares no tools, so nothing collides");
    let inputs = TurnInputs {
        policy: None,
        plugins: Some(RunPlugins {
            registry,
            context: context(),
        }),
    };

    let opened = tokio::time::timeout(RUN, fixture.job.run_open(&NoopEventSink, inputs))
        .await
        .expect("the first turn must settle");
    assert!(opened.is_ok(), "the first turn must run: {opened:?}");
    let second = tokio::time::timeout(
        RUN,
        fixture.job.run_next_turn(&NoopEventSink, "second message"),
    )
    .await
    .expect("the second turn must settle");
    assert!(second.is_ok(), "the second turn must run: {second:?}");

    let bodies = provider.bodies();
    assert_eq!(bodies.len(), 2, "one request per turn");
    let first = system_message(&bodies[0]);
    assert_eq!(
        first.matches(MARKER).count(),
        1,
        "the plugin's section appears exactly once: {first}"
    );
    let core = first
        .find(CORE_TAIL)
        .expect("the core prompt is in the system message");
    let section = first
        .find(MARKER)
        .expect("the plugin's section rode the run's prompt");
    assert!(
        section > core,
        "the section sits after the core prompt, not before it"
    );
    assert_eq!(
        system_message(&bodies[1]),
        first,
        "turn 2's system message repeats turn 1's byte for byte"
    );
    assert_eq!(
        composed.load(Ordering::Acquire),
        1,
        "the section is composed once for the run, not once per turn"
    );
}

/// Test 2 — an inactive plugin contributes zero bytes: its marker is absent
/// from the request body, and it is never asked for a section.
#[tokio::test]
async fn an_inactive_plugins_section_contributes_zero_bytes() {
    let provider = Scripted::start_with_script(vec![Reply::Text("the turn answered")]);
    let fixture = in_place("turn-section-inactive", &provider.endpoint).await;
    let silent = Arc::new(AtomicUsize::new(0));
    let registry = PluginRegistry::new(
        vec![
            Arc::new(SectionPlugin {
                name: "toolbox",
                active: true,
                marker: MARKER,
                composed: Arc::new(AtomicUsize::new(0)),
            }),
            Arc::new(SectionPlugin {
                name: "silent",
                active: false,
                marker: SILENT,
                composed: Arc::clone(&silent),
            }),
        ],
        &[],
    )
    .expect("the two plugins declare no tools, so nothing collides");
    let inputs = TurnInputs {
        policy: None,
        plugins: Some(RunPlugins {
            registry,
            context: context(),
        }),
    };

    let run = tokio::time::timeout(RUN, fixture.job.run_open(&NoopEventSink, inputs))
        .await
        .expect("the turn must settle");
    assert!(run.is_ok(), "the turn must run: {run:?}");

    let body = provider.bodies().remove(0);
    assert!(
        system_message(&body).contains(MARKER),
        "the active plugin's section rode the request"
    );
    assert!(
        !body.contains(SILENT),
        "an inactive plugin contributes zero bytes to the request body"
    );
    assert_eq!(
        silent.load(Ordering::Acquire),
        0,
        "an inactive plugin is never asked for a section"
    );
}
