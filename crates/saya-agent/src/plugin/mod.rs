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

use crate::{
    AgentEvent, AgentEventSink, AgentOutput, AnsweringLimits, ContextBlock, TokenUsage,
    ToolDefinition, ToolExecutor,
};

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
/// The four dimensions are the ones an answering surface's budget already
/// states — provider requests, tool calls, known reported tokens, and elapsed
/// time — so a database plugin can hand the core its whole question budget.
/// Each field is optional, and the registry keeps the tightest value any active
/// plugin asks for, so a plugin can only ever make a turn cheaper, never more
/// expensive than the core allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TurnPolicy {
    /// The most provider requests the turn may make.
    pub max_requests: Option<u32>,
    /// The most tool calls the turn may request.
    pub max_tool_calls: Option<u32>,
    /// The most reported tokens the turn may know it spent.
    pub max_known_tokens: Option<u32>,
    /// The longest the turn may run.
    pub max_elapsed: Option<Duration>,
}

impl TurnPolicy {
    /// The policy one answering surface's ceilings ask for: the three
    /// admission dimensions its [`AnsweringLimits`] states, plus the elapsed
    /// deadline the same table carries — which the answering engine leaves with
    /// the owner that arms it.
    ///
    /// A ceiling above `u32::MAX` saturates: a policy may only tighten.
    #[must_use]
    pub fn from_answering(limits: AnsweringLimits, max_elapsed: Option<Duration>) -> Self {
        Self {
            max_requests: ceiling(limits.max_requests),
            max_tool_calls: ceiling(limits.max_tool_calls),
            max_known_tokens: ceiling(limits.max_reported_tokens),
            max_elapsed,
        }
    }

    /// The tighter of two policies, dimension by dimension: a dimension only
    /// one of them states is that one, and a dimension both state is the
    /// smaller. A second arm therefore may only ever make a turn cheaper.
    #[must_use]
    pub fn tightest(self, other: Self) -> Self {
        Self {
            max_requests: tightest_of(self.max_requests, other.max_requests),
            max_tool_calls: tightest_of(self.max_tool_calls, other.max_tool_calls),
            max_known_tokens: tightest_of(self.max_known_tokens, other.max_known_tokens),
            max_elapsed: tightest_of(self.max_elapsed, other.max_elapsed),
        }
    }

    /// The same four dimensions as the answering engine states them: the
    /// admission ceilings a turn-scoped scope is armed with, and the elapsed
    /// deadline its owner arms. The inverse of [`Self::from_answering`].
    #[must_use]
    pub fn answering_limits(self) -> AnsweringLimits {
        AnsweringLimits {
            max_requests: self.max_requests.map(u64::from),
            max_tool_calls: self.max_tool_calls.map(u64::from),
            max_reported_tokens: self.max_known_tokens.map(u64::from),
        }
    }
}

/// One answering ceiling as the policy ceiling that carries it.
fn ceiling(limit: Option<u64>) -> Option<u32> {
    limit.map(|value| u32::try_from(value).unwrap_or(u32::MAX))
}

/// The tighter of two optional ceilings: unset means unbounded, so the set one
/// wins, and two set ones take the smaller.
fn tightest_of<T: Ord + Copy>(limit: Option<T>, other: Option<T>) -> Option<T> {
    match (limit, other) {
        (Some(limit), Some(other)) => Some(limit.min(other)),
        (Some(limit), None) => Some(limit),
        (None, other) => other,
    }
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

    /// The plugin's own section of the run's system prompt, or `None` — the
    /// default — for a plugin with nothing to state there.
    ///
    /// This is the run's prompt, not a turn's: the core appends it once, when it
    /// composes the prompt a run sends on every turn, so a section that changed
    /// between calls would make each turn's request prefix differ. State facts
    /// that hold for the whole run; anything per-turn belongs in `context`.
    fn system_section(&self, _ctx: &RunContext) -> Option<String> {
        None
    }

    /// Untrusted context blocks for this turn's prompt. The default contributes
    /// none — most plugins have nothing to recall.
    async fn context(&self, _ctx: &RunContext, _prompt: &str) -> Vec<ContextBlock> {
        Vec::new()
    }

    /// The single event reporting what this turn's [`Self::context`] supplied,
    /// or `None` — the default — for a plugin that supplies nothing to report.
    ///
    /// Read once per turn, after `context` has run and before the first
    /// provider request: this is where a plugin that recalled turns the recall
    /// it just performed into the one event a surface shows ("memory supplied ·
    /// 2 claims"), while the request is still being assembled. It describes
    /// *this* turn only — asking twice for one turn must not produce a second
    /// event — and the plugin's own recall receipt decides the outcome, so a
    /// surface can never be told more than the plugin actually did.
    fn knowledge_event(&self, _ctx: &RunContext) -> Option<AgentEvent> {
        None
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
    /// for none. A turn applies these from its start, whether or not it uses the
    /// plugin.
    fn turn_policy(&self, _ctx: &RunContext) -> Option<TurnPolicy> {
        None
    }

    /// The turn limits to arm the first time a turn USES this plugin — the
    /// plugin's context reached the turn, or a tool call was routed to it — or
    /// `None` — the default — to arm none.
    ///
    /// A turn that never uses the plugin spends nothing of this policy, which is
    /// the difference from [`Self::turn_policy`]: a database budget arms on the
    /// turn's first database use, not when the turn starts (`ref/05`, §2-§3).
    fn policy_on_use(&self, _ctx: &RunContext) -> Option<TurnPolicy> {
        None
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
