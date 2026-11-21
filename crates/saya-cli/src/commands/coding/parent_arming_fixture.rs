//! The plugin the arming tests state, and the turn they drive.
//!
//! One plugin: a policy it asks for only when a turn uses it
//! (`policy_on_use`), one echo tool it delivers, and a context block it puts in
//! the turn's prompt only when the test asks it to. Either of the last two is a
//! use of the plugin, so either arms the policy.

#![cfg(unix)]

use super::{RunPlugins, TurnInputs};
use crate::commands::coding::{CodingError, NativeCodingJob};
use saya_agent::{
    ActivePlugins, AgentOutput, ContextBlock, LocalStateEffect, NoopEventSink, PluginRegistry,
    RunContext, RunPlugin, RunSurface, ToolConcurrency, ToolDefinition, ToolEffect, ToolError,
    ToolExecutionContext, ToolExecutor, TurnPolicy,
};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

/// Load ceiling, not a product bound: how long one scripted turn may take.
pub(super) const RUN: Duration = Duration::from_secs(120);

/// The one tool the test plugin delivers, and the one it declares.
pub(super) const TOOL: &str = "plugin_echo";
const DECLARED: &[&str] = &[TOOL];

/// The context block a contributing plugin puts in the turn's prompt.
pub(super) const CONTEXT_BODY: &str = "plugin-context-for-this-turn";

/// The policy every test's plugin arms when its turn first uses it: one
/// request, so one more than the first counted request is refused.
pub(super) fn one_request() -> TurnPolicy {
    TurnPolicy {
        max_requests: Some(1),
        ..TurnPolicy::default()
    }
}

/// The context every test's run activates its plugin for.
fn chat() -> RunContext {
    RunContext {
        surface: RunSurface::Chat,
        database_profiles: 0,
    }
}

/// A plugin that delivers one echo tool, states a policy on use, and puts a
/// context block in the turn's prompt only when the test asks it to.
pub(super) struct TestPlugin {
    policy: TurnPolicy,
    contributes_context: bool,
    executed: Arc<AtomicUsize>,
}

impl TestPlugin {
    pub(super) fn new(policy: TurnPolicy, contributes_context: bool) -> Self {
        Self {
            policy,
            contributes_context,
            executed: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// How many of this plugin's tool calls the core routed to it.
    pub(super) fn executed(&self) -> usize {
        self.executed.load(Ordering::Acquire)
    }
}

/// This one plugin's registry, checked for name collisions as the run's is.
fn registry(plugin: Arc<TestPlugin>) -> PluginRegistry {
    PluginRegistry::new(vec![plugin], &[])
        .expect("the test plugin declares no name the empty core list owns")
}

/// The active plugins one run composes out of this plugin.
pub(super) fn active(plugin: Arc<TestPlugin>) -> Arc<ActivePlugins> {
    Arc::new(registry(plugin).active(&chat()))
}

#[async_trait::async_trait]
impl RunPlugin for TestPlugin {
    fn name(&self) -> &'static str {
        "arming-plugin"
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
        Some(Arc::new(Echo(Arc::clone(&self.executed))))
    }

    async fn context(&self, _ctx: &RunContext, _prompt: &str) -> Vec<ContextBlock> {
        if !self.contributes_context {
            return Vec::new();
        }
        vec![ContextBlock {
            label: "arming plugin".into(),
            body: CONTEXT_BODY.into(),
            truncated: false,
        }]
    }

    fn policy_on_use(&self, _ctx: &RunContext) -> Option<TurnPolicy> {
        Some(self.policy)
    }
}

/// The plugin's own executor: answers with a marker and counts the call.
struct Echo(Arc<AtomicUsize>);

#[async_trait::async_trait]
impl ToolExecutor for Echo {
    async fn execute_with_context(
        &self,
        _name: &str,
        _arguments: serde_json::Value,
        _context: ToolExecutionContext,
    ) -> Result<serde_json::Value, ToolError> {
        self.0.fetch_add(1, Ordering::AcqRel);
        Ok(json!({"echo": "plugin-echo-ok"}))
    }

    async fn execute(
        &self,
        _name: &str,
        _arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        self.0.fetch_add(1, Ordering::AcqRel);
        Ok(json!({"echo": "plugin-echo-ok"}))
    }
}

/// The turn's inputs with one registry: this plugin, on the chat surface.
pub(super) fn inputs(plugin: Arc<TestPlugin>) -> TurnInputs {
    TurnInputs {
        policy: None,
        plugins: Some(RunPlugins {
            registry: registry(plugin),
            context: chat(),
        }),
        seed: Vec::new(),
    }
}

/// Runs the first turn of a fresh job, bounded so a wedged turn fails the test.
pub(super) async fn first_turn(
    job: &Arc<NativeCodingJob>,
    inputs: TurnInputs,
) -> Result<AgentOutput, CodingError> {
    tokio::time::timeout(RUN, job.run_open(&NoopEventSink, inputs))
        .await
        .expect("the turn must settle")
}
