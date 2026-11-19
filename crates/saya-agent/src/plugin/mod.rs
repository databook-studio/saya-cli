//! The `RunPlugin` contract: what a built-in plugin may contribute to a run,
//! and the registry that collects contributions from the active ones.
//!
//! One run core serves every surface. The database side of the chat — DB tools,
//! the question budget, clarification, memory recall, learning — moves into a
//! built-in plugin that the core loads on every surface, so that a surface's
//! differences live in a plugin, not in a second copy of the loop. This module
//! declares only the contract and its registry; nothing here is wired into the
//! run path yet.

mod registry;

pub use registry::{ActivePlugins, PluginError, PluginRegistry};

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::{AgentEventSink, AgentOutput, ContextBlock, TokenUsage, ToolDefinition, ToolExecutor};

/// Which surface is running — the fact a plugin keys its activation on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunSurface {
    /// The interactive chat (TUI or REPL).
    Chat,
    /// A one-shot headless run.
    Headless,
    /// A background run nobody is watching.
    Background,
    /// A helper run a tool starts for itself (a child, a delegated turn).
    Helper,
}

/// The plain data a plugin reads to decide whether and what it contributes.
///
/// Deliberately small and copyable: a plugin must not be able to steer the run
/// by handing state back through this type. Fields are added only when a test
/// needs one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RunContext {
    /// The surface this run serves.
    pub surface: RunSurface,
    /// How many database profiles the run was resolved with. Zero means the
    /// surface has no database, which is what a database plugin keys on.
    pub database_profiles: usize,
}

/// How a turn ended, and what it produced. Minimal on purpose: unit 2.4 extends
/// it with the fields a budget plugin needs to report what stopped the turn.
///
/// The prompt and the output are borrowed from the turn — a plugin observes
/// them, never owns them. `Eq` and `Default` go with the borrowed output, whose
/// events carry a JSON value; `Copy` stays, since every field is `Copy`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TurnOutcome<'a> {
    /// The turn produced an answer for the user.
    pub answered: bool,
    /// The user cancelled the turn before it answered.
    pub cancelled: bool,
    /// The user's question for this turn.
    pub prompt: &'a str,
    /// What the turn produced, or `None` when the answering call failed —
    /// there is nothing for a post-turn hook to read.
    pub output: Option<&'a AgentOutput>,
}

/// The turn-level limits a plugin asks the core to enforce.
///
/// Minimal on purpose: unit 2.4 extends it (the question budget, elapsed-time
/// and request ceilings a database turn needs). Each field is optional, and the
/// registry keeps the tightest value any active plugin asks for, so a plugin can
/// only ever make a turn cheaper, never more expensive than the core allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TurnPolicy {
    /// The most provider requests the turn may make.
    pub max_requests: Option<u32>,
    /// The longest the turn may run.
    pub max_elapsed: Option<Duration>,
}

/// A built-in plugin: definitions, an executor, untrusted context, turn hooks,
/// and a policy — every contribution optional.
///
/// A plugin never decides permissions, approvals, or safety: a tool it delivers
/// still goes through the same executor gates as a core tool, and the context it
/// returns is rendered as untrusted data. The plugin decides *what it delivers
/// for this run*, not what is allowed.
#[async_trait]
pub trait RunPlugin: Send + Sync {
    /// The plugin's stable name, used in diagnostics and collision errors.
    fn name(&self) -> &'static str;

    /// Every tool name this plugin can ever deliver, on any surface and in any
    /// context — not only the ones it offers right now.
    ///
    /// The registry checks name collisions against this list once, at
    /// construction, and filters each plugin's definitions by it. A name a
    /// plugin only offers when a database profile is configured therefore still
    /// cannot shadow a core tool.
    fn declared_tools(&self) -> &'static [&'static str];

    /// Whether this plugin takes part in a run of `ctx`. An inactive plugin
    /// contributes nothing at all — no definitions, no route, no context, no
    /// policy, no hooks.
    fn activate(&self, ctx: &RunContext) -> bool;

    /// The tool definitions this plugin delivers for `ctx`. The core appends
    /// them after its own tools.
    fn definitions(&self, ctx: &RunContext) -> Vec<ToolDefinition>;

    /// The executor that serves this plugin's tools, or `None` for a plugin
    /// that delivers only context and policy.
    fn executor(&self) -> Option<Arc<dyn ToolExecutor>>;

    /// Untrusted context blocks for this turn's prompt. The default contributes
    /// none — most plugins have nothing to recall.
    async fn context(&self, _ctx: &RunContext, _prompt: &str) -> Vec<ContextBlock> {
        Vec::new()
    }

    /// Called once when a turn starts, before the first request. The default
    /// does nothing.
    async fn on_turn_start(&self, _ctx: &RunContext) {}

    /// Called once when a turn ends, with its outcome and the run's event sink.
    /// The default does nothing; an implementation observes, records, and may
    /// return the usage its own post-turn work spent, which the registry sums
    /// into the turn's learning total. It never changes the outcome.
    async fn on_turn_finish(
        &self,
        _ctx: &RunContext,
        _outcome: &TurnOutcome<'_>,
        _sink: &dyn AgentEventSink,
    ) -> Option<TokenUsage> {
        None
    }

    /// The turn limits this plugin asks for, or `None` — the default — to ask
    /// for none.
    fn turn_policy(&self, _ctx: &RunContext) -> Option<TurnPolicy> {
        None
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
