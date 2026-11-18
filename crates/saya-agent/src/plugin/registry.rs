//! The registry that collects the active plugins' contributions for one run.
//!
//! A tool name is global to a run, so the registry refuses a duplicate once, at
//! construction, rather than letting two owners answer one name. Everything
//! after that is per-run: which plugins activate, what they deliver, and what
//! the core may spend on their behalf.

use std::sync::Arc;

use super::{RunContext, RunPlugin, RunSurface, TurnOutcome, TurnPolicy};
use crate::history_context::escape_block_text;
use crate::{ContextBlock, ToolDefinition, ToolExecutor};

/// The surfaces a name is checked against. A collision on any surface is
/// refused, because the name a plugin declares is global to the run — a plugin
/// that only activates on chat still owns its tool names everywhere.
const PROBED_SURFACES: [RunSurface; 4] = [
    RunSurface::Chat,
    RunSurface::Headless,
    RunSurface::Background,
    RunSurface::Helper,
];

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
    /// runtime surprise.
    pub fn new(
        plugins: Vec<Arc<dyn RunPlugin>>,
        core_tool_names: &[&str],
    ) -> Result<Self, PluginError> {
        let mut claimed: Vec<String> = Vec::new();
        for plugin in &plugins {
            for tool in declared_tool_names(plugin) {
                let owned = core_tool_names.contains(&tool.as_str()) || claimed.contains(&tool);
                if owned {
                    return Err(PluginError::NameCollision {
                        plugin: plugin.name(),
                        tool,
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

/// Every tool name `plugin` declares, across all surfaces, deduplicated.
fn declared_tool_names(plugin: &Arc<dyn RunPlugin>) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for surface in PROBED_SURFACES {
        let ctx = RunContext {
            surface,
            database_profiles: 0,
        };
        for definition in plugin.definitions(&ctx) {
            if !names.contains(&definition.name) {
                names.push(definition.name);
            }
        }
    }
    names
}

/// The active plugins for one run, plus the context they activated for.
pub struct ActivePlugins {
    ctx: RunContext,
    plugins: Vec<Arc<dyn RunPlugin>>,
}

impl ActivePlugins {
    /// The definitions to append after the core tools, in registration order.
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.plugins
            .iter()
            .flat_map(|plugin| plugin.definitions(&self.ctx))
            .collect()
    }

    /// The executor of the plugin that owns `tool_name`, or `None` when no
    /// active plugin declares it.
    pub fn route(&self, tool_name: &str) -> Option<Arc<dyn ToolExecutor>> {
        self.plugins
            .iter()
            .find(|plugin| {
                plugin
                    .definitions(&self.ctx)
                    .iter()
                    .any(|definition| definition.name == tool_name)
            })
            .and_then(|plugin| plugin.executor())
    }

    /// The active plugins' context blocks, in registration order, bounded by
    /// `budget_bytes`.
    ///
    /// A block that would exceed the remaining budget is dropped whole — never
    /// truncated mid-block, because half a contract reads as a whole one — and
    /// the blocks after it are still offered what is left, so one oversized
    /// block cannot starve a smaller one behind it.
    pub async fn context(
        &self,
        ctx: &RunContext,
        prompt: &str,
        budget_bytes: usize,
    ) -> Vec<ContextBlock> {
        let mut remaining = budget_bytes;
        let mut kept = Vec::new();
        for plugin in &self.plugins {
            for block in plugin.context(ctx, prompt).await {
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
                Some(tightest) => TurnPolicy {
                    max_requests: tightest_of(tightest.max_requests, asked.max_requests),
                    max_elapsed: tightest_of(tightest.max_elapsed, asked.max_elapsed),
                },
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

    /// Calls every active plugin's `on_turn_finish` once, in order.
    pub async fn on_turn_finish(&self, outcome: &TurnOutcome) {
        for plugin in &self.plugins {
            plugin.on_turn_finish(&self.ctx, outcome).await;
        }
    }
}

/// The tighter of two optional limits: the smaller value, and the one that is
/// set when only one side is.
fn tightest_of<T: Ord + Copy>(limit: Option<T>, other: Option<T>) -> Option<T> {
    match (limit, other) {
        (Some(limit), Some(other)) => Some(limit.min(other)),
        (Some(limit), None) => Some(limit),
        (None, other) => other,
    }
}

/// The block's cost against a budget: the rendered text after the escaping the
/// request build applies — the same post-escape accounting `turn_bytes` uses for
/// a context block. The delimiters and the `source:` label are framing the
/// budget does not charge a block for.
fn block_bytes(block: &ContextBlock) -> usize {
    escape_block_text(&block.body).len()
}
