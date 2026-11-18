//! The fake plugin the contract tests drive: every contribution is set by the
//! test, and the hook calls the registry makes are recorded for it to observe.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use super::super::{RunContext, RunPlugin, RunSurface, TurnOutcome, TurnPolicy};
use crate::{
    ContextBlock, LocalStateEffect, ToolConcurrency, ToolDefinition, ToolEffect, ToolError,
    ToolExecutor,
};

/// A chat run with a database attached.
pub(super) const CHAT: RunContext = RunContext {
    surface: RunSurface::Chat,
    database_profiles: 1,
};

/// An executor that names itself, so a test can tell which plugin served a
/// routed tool.
struct FakeExecutor {
    label: &'static str,
}

#[async_trait]
impl ToolExecutor for FakeExecutor {
    async fn execute(
        &self,
        _name: &str,
        _arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        Ok(serde_json::json!({ "executor": self.label }))
    }
}

/// A plugin whose contributions are whatever the test set, and which records
/// the hook calls made on it.
pub(super) struct FakePlugin {
    name: &'static str,
    active: bool,
    tool_names: Vec<&'static str>,
    executor: Option<Arc<dyn ToolExecutor>>,
    blocks: Vec<ContextBlock>,
    policy: Option<TurnPolicy>,
    calls: Arc<Mutex<Vec<String>>>,
}

impl FakePlugin {
    pub(super) fn new(name: &'static str, active: bool) -> Self {
        Self {
            name,
            active,
            tool_names: Vec::new(),
            executor: None,
            blocks: Vec::new(),
            policy: None,
            calls: Arc::default(),
        }
    }

    pub(super) fn tools(mut self, names: &[&'static str]) -> Self {
        self.tool_names = names.to_vec();
        self
    }

    pub(super) fn serving(mut self, label: &'static str) -> Self {
        self.executor = Some(Arc::new(FakeExecutor { label }));
        self
    }

    pub(super) fn blocks(mut self, blocks: Vec<ContextBlock>) -> Self {
        self.blocks = blocks;
        self
    }

    pub(super) fn policy(mut self, policy: TurnPolicy) -> Self {
        self.policy = Some(policy);
        self
    }

    /// Records this plugin's hook calls into `calls`, so several plugins can
    /// share one log and their order is observable.
    pub(super) fn logging(mut self, calls: Arc<Mutex<Vec<String>>>) -> Self {
        self.calls = calls;
        self
    }

    fn record(&self, phase: &str) {
        self.calls
            .lock()
            .unwrap()
            .push(format!("{}:{phase}", self.name));
    }
}

#[async_trait]
impl RunPlugin for FakePlugin {
    fn name(&self) -> &'static str {
        self.name
    }

    fn activate(&self, _ctx: &RunContext) -> bool {
        self.active
    }

    fn definitions(&self, _ctx: &RunContext) -> Vec<ToolDefinition> {
        self.tool_names
            .iter()
            .map(|name| definition(name))
            .collect()
    }

    fn executor(&self) -> Option<Arc<dyn ToolExecutor>> {
        self.executor.clone()
    }

    async fn context(&self, _ctx: &RunContext, _prompt: &str) -> Vec<ContextBlock> {
        self.blocks.clone()
    }

    async fn on_turn_start(&self, _ctx: &RunContext) {
        self.record("start");
    }

    async fn on_turn_finish(&self, _ctx: &RunContext, outcome: &TurnOutcome) {
        self.record(if outcome.answered { "finish" } else { "end" });
    }

    fn turn_policy(&self, _ctx: &RunContext) -> Option<TurnPolicy> {
        self.policy
    }
}

/// A minimal read-only definition carrying `name`.
fn definition(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: format!("fake definition for {name}"),
        read_only: true,
        parameters: serde_json::json!({ "type": "object", "properties": {} }),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::None,
        },
        concurrency: ToolConcurrency::Serial,
        completion: None,
    }
}

/// A block whose body is exactly `bytes` bytes of one ASCII character, so a
/// test's budget arithmetic is the block's own size.
pub(super) fn block(label: &str, bytes: usize) -> ContextBlock {
    ContextBlock {
        label: label.to_string(),
        body: "x".repeat(bytes),
        truncated: false,
    }
}

pub(super) fn labels(blocks: &[ContextBlock]) -> Vec<&str> {
    blocks.iter().map(|block| block.label.as_str()).collect()
}

pub(super) fn answered() -> TurnOutcome {
    TurnOutcome {
        answered: true,
        cancelled: false,
    }
}
