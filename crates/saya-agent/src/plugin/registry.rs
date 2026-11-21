//! The registry that collects the active plugins' contributions for one run.
//!
//! A tool name is global to a run, so the registry refuses a duplicate once, at
//! construction, rather than letting two owners answer one name. Everything
//! after that is per-run: which plugins activate, what they deliver, and what
//! the core may spend on their behalf.

use std::sync::Arc;

use super::{RunContext, RunPlugin, TurnOutcome, TurnPolicy};
use crate::history_context::escape_block_text;
use crate::loop_runner::add_usage;
use crate::{AgentEventSink, ContextBlock, TokenUsage, ToolDefinition, ToolExecutor};

/// Why a registry refused to be built. A tool name is global to a run, so a
/// plugin may not take one the core already owns, or one another plugin owns.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PluginError {
    /// `plugin` declares `tool`, which a core tool or an earlier plugin owns.
    #[error("plugin `{plugin}` declares tool `{tool}`, which is already owned")]
    NameCollision {
        /// The plugin that declared the duplicate.
        plugin: &'static str,
        /// The tool name that was already owned.
        tool: String,
    },
}

/// The plugins a run loaded, checked for name collisions once.
pub struct PluginRegistry {
    plugins: Vec<Arc<dyn RunPlugin>>,
}

impl std::fmt::Debug for PluginRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PluginRegistry")
            .field(
                "plugins",
                &self
                    .plugins
                    .iter()
                    .map(|plugin| plugin.name())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl PluginRegistry {
    /// Builds the registry from the run's plugins and the core tool names.
    ///
    /// Refuses the whole registry when a plugin declares a name a core tool
    /// owns or another plugin already declared: an ambiguous name would let two
    /// owners answer one tool call, so it is a construction failure, not a
    /// runtime surprise. The check reads every name a plugin can ever deliver,
    /// so a tool offered only once a database profile is configured is held to
    /// the same rule as one offered on every run.
    pub fn new(
        plugins: Vec<Arc<dyn RunPlugin>>,
        core_tool_names: &[&str],
    ) -> Result<Self, PluginError> {
        let mut claimed: Vec<&'static str> = Vec::new();
        for plugin in &plugins {
            for tool in plugin.declared_tools() {
                if core_tool_names.contains(tool) || claimed.contains(tool) {
                    return Err(PluginError::NameCollision {
                        plugin: plugin.name(),
                        tool: tool.to_string(),
                    });
                }
                claimed.push(tool);
            }
        }
        Ok(Self { plugins })
    }

    /// The plugins whose `activate` is true for `ctx`, in registration order.
    pub fn active(&self, ctx: &RunContext) -> ActivePlugins {
        ActivePlugins {
            ctx: *ctx,
            plugins: self
                .plugins
                .iter()
                .filter(|plugin| plugin.activate(ctx))
                .cloned()
                .collect(),
        }
    }
}

/// The active plugins for one run, plus the context they activated for.
pub struct ActivePlugins {
    ctx: RunContext,
    plugins: Vec<Arc<dyn RunPlugin>>,
}

impl ActivePlugins {
    /// The definitions to append after the core tools, in registration order.
    ///
    /// A definition whose name the plugin never declared is dropped: the name
    /// was checked against the core and the other plugins at construction, so a
    /// definition outside that list is a plugin bug, not something the core may
    /// append. A debug build asserts against it rather than filtering quietly.
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        let mut kept = Vec::new();
        for plugin in &self.plugins {
            let declared = plugin.declared_tools();
            for definition in plugin.definitions(&self.ctx) {
                let in_declared = declared.contains(&definition.name.as_str());
                debug_assert!(
                    in_declared,
                    "plugin `{}` offers tool `{}`, which it never declared",
                    plugin.name(),
                    definition.name
                );
                if in_declared {
                    kept.push(definition);
                }
            }
        }
        kept
    }

    /// The executor of the plugin that owns `tool_name`, or `None` when no
    /// active plugin declares it or the owner does not offer it in this
    /// context.
    ///
    /// Ownership is decided by `declared_tools`, so the route is the name the
    /// registry checked at construction; the second check is what keeps a tool
    /// the plugin only delivers under some other context from being routed here.
    pub fn route(&self, tool_name: &str) -> Option<Arc<dyn ToolExecutor>> {
        let owner = self
            .plugins
            .iter()
            .find(|plugin| plugin.declared_tools().contains(&tool_name))?;
        let offered = owner
            .definitions(&self.ctx)
            .iter()
            .any(|definition| definition.name == tool_name);
        if !offered {
            return None;
        }
        owner.executor()
    }

    /// The active plugins' sections of the run's system prompt, in
    /// registration order; a plugin that states none contributes nothing.
    ///
    /// The core appends these once, after its own prompt, when it composes the
    /// prompt a run sends on every turn: they belong to the run, not to a turn,
    /// so every turn carries the same bytes and the request prefix stays stable.
    pub fn system_sections(&self) -> Vec<String> {
        self.plugins
            .iter()
            .filter_map(|plugin| plugin.system_section(&self.ctx))
            .collect()
    }

    /// The active plugins' context blocks, in registration order, bounded by
    /// `budget_bytes`.
    ///
    /// A block that would exceed the remaining budget is dropped whole — never
    /// truncated mid-block, because half a contract reads as a whole one — and
    /// the blocks after it are still offered what is left, so one oversized
    /// block cannot starve a smaller one behind it.
    pub async fn context(&self, prompt: &str, budget_bytes: usize) -> Vec<ContextBlock> {
        let mut remaining = budget_bytes;
        let mut kept = Vec::new();
        for plugin in &self.plugins {
            for block in plugin.context(&self.ctx, prompt).await {
                let cost = block_bytes(&block);
                if cost > remaining {
                    continue;
                }
                remaining -= cost;
                kept.push(block);
            }
        }
        kept
    }

    /// The tightest turn policy across the active plugins, or `None` when none
    /// asks for one. Absence is not a zero-request turn: a field no plugin sets
    /// stays unset.
    pub fn turn_policy(&self) -> Option<TurnPolicy> {
        let mut policy: Option<TurnPolicy> = None;
        for plugin in &self.plugins {
            let Some(asked) = plugin.turn_policy(&self.ctx) else {
                continue;
            };
            policy = Some(match policy {
                Some(tightest) => tightest.tightest(asked),
                None => asked,
            });
        }
        policy
    }

    /// Calls every active plugin's `on_turn_start` once, in order.
    pub async fn on_turn_start(&self) {
        for plugin in &self.plugins {
            plugin.on_turn_start(&self.ctx).await;
        }
    }

    /// Calls every active plugin's `on_turn_finish` once, in order, and sums the
    /// usage each reported. `None` when no plugin reported any — absent is not
    /// a reported zero.
    pub async fn on_turn_finish(
        &self,
        outcome: &TurnOutcome<'_>,
        sink: &dyn AgentEventSink,
    ) -> Option<TokenUsage> {
        let mut total: Option<TokenUsage> = None;
        for plugin in &self.plugins {
            let Some(reported) = plugin.on_turn_finish(&self.ctx, outcome, sink).await else {
                continue;
            };
            match total.as_mut() {
                Some(sum) => add_usage(sum, reported),
                None => total = Some(reported),
            }
        }
        total
    }
}

/// The block's cost against a budget: the rendered text after the escaping the
/// request build applies — the same post-escape accounting `turn_bytes` uses for
/// a context block. The delimiters and the `source:` label are framing the
/// budget does not charge a block for.
fn block_bytes(block: &ContextBlock) -> usize {
    escape_block_text(&block.body).len()
}
