//! A root turn consults the plugin registry its caller supplies (phase-4 unit
//! 4.0b).
//!
//! The registry is the door the helper surfaces already compose through: the
//! active plugin's tools join the core's, after them; a call to one is routed
//! to that plugin; and the plugin's context and turn-end hook are called once
//! each per turn. With no registry supplied — every job surface today — the run
//! consults nothing.

#![cfg(unix)]

use super::super::parent_turn_fixture::in_place_writing;
use super::super::parent_turn_provider::{Reply, Scripted};
use super::super::{RunPlugins, TurnInputs};
use super::RUN;
use saya_agent::{
    AgentEventSink, ContextBlock, LocalStateEffect, NoopEventSink, PluginRegistry, RunContext,
    RunPlugin, RunSurface, TokenUsage, ToolConcurrency, ToolDefinition, ToolEffect, ToolError,
    ToolExecutionContext, ToolExecutor, TurnOutcome,
};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

/// The one tool the test plugin delivers, and the one it declares.
const TOOL: &str = "plugin_echo";
const DECLARED: &[&str] = &[TOOL];

/// What the core asked of the plugin, counted.
#[derive(Default)]
struct Calls {
    context: AtomicUsize,
    finished: AtomicUsize,
    executed: AtomicUsize,
}

impl Calls {
    fn of(counter: &AtomicUsize) -> usize {
        counter.load(Ordering::Acquire)
    }
}

/// A plugin that delivers one echo tool, one context block and one turn hook.
struct TestPlugin(Arc<Calls>);

#[async_trait::async_trait]
impl RunPlugin for TestPlugin {
    fn name(&self) -> &'static str {
        "test-plugin"
    }

    fn declared_tools(&self) -> &'static [&'static str] {
        DECLARED
    }

    fn activate(&self, _ctx: &RunContext) -> bool {
        true
    }

    fn definitions(&self, _ctx: &RunContext) -> Vec<ToolDefinition> {
        vec![ToolDefinition {
            name: TOOL.into(),
            description: "echoes the arguments it is given".into(),
            read_only: true,
            parameters: json!({"type": "object", "properties": {}}),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::None,
            },
            concurrency: ToolConcurrency::Serial,
            completion: None,
        }]
    }

    fn executor(&self) -> Option<Arc<dyn ToolExecutor>> {
        Some(Arc::new(Echo(Arc::clone(&self.0))))
    }

    async fn context(&self, _ctx: &RunContext, _prompt: &str) -> Vec<ContextBlock> {
        self.0.context.fetch_add(1, Ordering::AcqRel);
        vec![ContextBlock {
            label: "test plugin".into(),
            body: "plugin-context-once".into(),
            truncated: false,
        }]
    }

    async fn on_turn_finish(
        &self,
        _ctx: &RunContext,
        _outcome: &TurnOutcome<'_>,
        _sink: &dyn AgentEventSink,
    ) -> Option<TokenUsage> {
        self.0.finished.fetch_add(1, Ordering::AcqRel);
        None
    }
}

/// The plugin's own executor: it answers with a marker the test can find in the
/// request that followed the call.
struct Echo(Arc<Calls>);

#[async_trait::async_trait]
impl ToolExecutor for Echo {
    async fn execute_with_context(
        &self,
        _name: &str,
        _arguments: serde_json::Value,
        _context: ToolExecutionContext,
    ) -> Result<serde_json::Value, ToolError> {
        self.0.executed.fetch_add(1, Ordering::AcqRel);
        Ok(json!({"echo": "plugin-echo-ok"}))
    }

    async fn execute(
        &self,
        _name: &str,
        _arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        self.0.executed.fetch_add(1, Ordering::AcqRel);
        Ok(json!({"echo": "plugin-echo-ok"}))
    }
}

/// The tool names a provider request advertises, in order.
fn advertised(body: &str) -> Vec<String> {
    let body: serde_json::Value = serde_json::from_str(body).expect("a request body is JSON");
    body["tools"]
        .as_array()
        .expect("a request advertises its tools")
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str().map(str::to_owned))
        .collect()
}

/// Test 4 — with a registry holding a test plugin, its tool is advertised after
/// the core tools, its context and turn-end hook are called once per turn, and a
/// call to its tool is routed to it.
#[tokio::test]
async fn the_root_turn_consults_the_registry_it_is_given() {
    let provider = Scripted::start_with_script(vec![
        Reply::Call {
            name: TOOL,
            arguments: json!({"text": "hello"}),
        },
        Reply::Text("the first turn answered"),
        Reply::Text("the second turn answered"),
    ]);
    let fixture = in_place_writing("turn-plugins", &provider.endpoint).await;
    let calls = Arc::new(Calls::default());
    let registry = PluginRegistry::new(vec![Arc::new(TestPlugin(Arc::clone(&calls)))], &[])
        .expect("the test plugin declares no name the empty core list owns");
    let inputs = TurnInputs {
        policy: None,
        plugins: Some(RunPlugins {
            registry,
            context: RunContext {
                surface: RunSurface::Chat,
                database_profiles: 0,
            },
            primary: None,
        }),
        seed: Vec::new(),
        ..TurnInputs::default()
    };

    let first = tokio::time::timeout(RUN, fixture.job.run_open(&NoopEventSink, inputs))
        .await
        .expect("the first turn must settle");
    assert!(first.is_ok(), "the first turn must run: {first:?}");
    let bodies = provider.bodies();
    let names = advertised(&bodies[0]);
    assert_eq!(
        names.last().map(String::as_str),
        Some(TOOL),
        "the plugin's tool is advertised after the core tools: {names:?}"
    );
    assert!(
        names.len() > 1,
        "the core tools come first, and there are some: {names:?}"
    );
    assert!(
        bodies[0].contains("plugin-context-once"),
        "the plugin's context block rode the turn's request"
    );
    assert_eq!(
        Calls::of(&calls.executed),
        1,
        "the call to the plugin's tool was routed to the plugin"
    );
    assert!(
        bodies[1].contains("plugin-echo-ok"),
        "the plugin's answer is the tool result the model saw next"
    );

    let second = tokio::time::timeout(
        RUN,
        fixture.job.run_next_turn(&NoopEventSink, "second message"),
    )
    .await
    .expect("the second turn must settle");
    assert!(second.is_ok(), "the second turn must run: {second:?}");
    assert_eq!(
        Calls::of(&calls.context),
        2,
        "the plugin's context is asked for exactly once per turn"
    );
    assert_eq!(
        Calls::of(&calls.finished),
        2,
        "and its turn-end hook called exactly once per turn"
    );
}
