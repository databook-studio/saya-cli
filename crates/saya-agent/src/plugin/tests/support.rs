//! The fake plugin the contract tests drive: every contribution is set by the
//! test, and the hook calls the registry makes are recorded for it to observe.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use super::super::{RunContext, RunPlugin, RunSurface, TurnOutcome, TurnPolicy};
use crate::{
    AgentEventSink, ContextBlock, LocalStateEffect, TokenUsage, ToolConcurrency, ToolDefinition,
    ToolEffect, ToolError, ToolExecutor,
};

/// A chat run with a database attached.
pub(super) const CHAT: RunContext = RunContext {
    surface: RunSurface::Chat,
    database_profiles: 1,
};

/// A chat run with no database, so a database plugin offers none of its tools.
pub(super) const NO_DB: RunContext = RunContext {
    surface: RunSurface::Chat,
    database_profiles: 0,
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
    declared: &'static [&'static str],
    offered: &'static [&'static str],
    profile_offered: &'static [&'static str],
    executor: Option<Arc<dyn ToolExecutor>>,
    blocks: Vec<ContextBlock>,
    section: Option<String>,
    policy: Option<TurnPolicy>,
    usage: Option<TokenUsage>,
    calls: Arc<Mutex<Vec<String>>>,
}

impl FakePlugin {
    pub(super) fn new(name: &'static str, active: bool) -> Self {
        Self {
            name,
            active,
            declared: &[],
            offered: &[],
            profile_offered: &[],
            executor: None,
            blocks: Vec::new(),
            section: None,
            policy: None,
            usage: None,
            calls: Arc::default(),
        }
    }

    /// Offers these tools in every context, and declares them.
    pub(super) fn tools(mut self, names: &'static [&'static str]) -> Self {
        self.declared = names;
        self.offered = names;
        self
    }

    /// Declares these tools but offers none of them here — the plugin can
    /// deliver them, just not for this test's context.
    pub(super) fn declaring(mut self, names: &'static [&'static str]) -> Self {
        self.declared = names;
        self
    }

    /// Offers these tools, declaring whatever was declared before — which lets
    /// a test show a plugin offering a name it never declared.
    pub(super) fn offering(mut self, names: &'static [&'static str]) -> Self {
        self.offered = names;
        self
    }

    /// Declares these tools and offers them only when a database profile is
    /// configured — what a database plugin does.
    pub(super) fn profile_tools(mut self, names: &'static [&'static str]) -> Self {
        self.declared = names;
        self.profile_offered = names;
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

    /// States this section of the run's system prompt.
    pub(super) fn section(mut self, body: &str) -> Self {
        self.section = Some(body.to_owned());
        self
    }

    pub(super) fn policy(mut self, policy: TurnPolicy) -> Self {
        self.policy = Some(policy);
        self
    }

    /// The usage this plugin reports from its turn-finish hook.
    pub(super) fn usage(mut self, usage: TokenUsage) -> Self {
        self.usage = Some(usage);
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

    fn declared_tools(&self) -> &'static [&'static str] {
        self.declared
    }

    fn definitions(&self, ctx: &RunContext) -> Vec<ToolDefinition> {
        let mut names: Vec<&'static str> = self.offered.to_vec();
        if ctx.database_profiles > 0 {
            names.extend_from_slice(self.profile_offered);
        }
        names.into_iter().map(definition).collect()
    }

    fn executor(&self) -> Option<Arc<dyn ToolExecutor>> {
        self.executor.clone()
    }

    async fn context(&self, _ctx: &RunContext, _prompt: &str) -> Vec<ContextBlock> {
        self.blocks.clone()
    }

    fn system_section(&self, _ctx: &RunContext) -> Option<String> {
        self.section.clone()
    }

    async fn on_turn_start(&self, _ctx: &RunContext) {
        self.record("start");
    }

    async fn on_turn_finish(
        &self,
        _ctx: &RunContext,
        outcome: &TurnOutcome<'_>,
        _sink: &dyn AgentEventSink,
    ) -> Option<TokenUsage> {
        self.record(if outcome.answered { "finish" } else { "end" });
        self.usage
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

/// The names of a `definitions()` result, in order.
pub(super) fn definition_names(definitions: Vec<ToolDefinition>) -> Vec<String> {
    definitions
        .into_iter()
        .map(|definition| definition.name)
        .collect()
}

pub(super) fn answered() -> TurnOutcome<'static> {
    TurnOutcome {
        answered: true,
        cancelled: false,
        prompt: "a question about orders",
        output: None,
    }
}
