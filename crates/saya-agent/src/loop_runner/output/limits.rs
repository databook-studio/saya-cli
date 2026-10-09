use crate::{AgentEvent, ProviderError, ToolMetadata};
use thiserror::Error;

#[derive(Debug, Clone, Copy)]
pub struct AgentLimits {
    /// Ceiling on the number of provider turns, or `None` to run without a
    /// turn ceiling. `None` is the default: the loop stops when the model
    /// stops calling tools, when it is cancelled, or — when a ceiling is set
    /// — by salvaging the best answer from the work already done. There is no
    /// upper limit on a set value.
    pub max_turns: Option<usize>,
    /// Ceiling on the total number of tool calls across the whole run, or
    /// `None` for no ceiling. Same stopping policy as [`Self::max_turns`].
    pub max_tool_calls: Option<usize>,
    /// Ceiling on how many times one run may be re-instructed to continue after
    /// the provider capped a response mid-answer. `None` disables continuation
    /// entirely (the truncation error surfaces as it does today).
    ///
    /// Defaults to `Some(3)` — deliberately not `None`, unlike the other two
    /// ceilings. The other ceilings are "unbounded until someone asks for a
    /// bound"; this one is "bounded until someone measures a reason to raise
    /// it", because each continuation re-sends the trimmed history and cost
    /// grows roughly quadratically.
    pub max_continuations: Option<u32>,
    /// Whether the loop may execute tools that declare
    /// [`LocalStateEffect::WriteCandidate`](crate::LocalStateEffect::WriteCandidate).
    /// Defaults to **not permitted**: a tool that can write a candidate claim
    /// must not start writing merely because it was registered.
    pub permit_candidate_writes: bool,
    /// Whether the loop may execute tools that declare
    /// [`LocalStateEffect::WriteWorkspace`](crate::LocalStateEffect::WriteWorkspace).
    /// Defaults to **not permitted**: a tool that can write workspace files
    /// must not start writing merely because it was registered.
    pub permit_workspace_writes: bool,
    /// Whether the loop may execute tools that declare an external side
    /// effect without a per-call approval prompt. Defaults to **not
    /// permitted**: the misconfiguration guard denies such a tool exactly as
    /// before, so an interactive `ask` turn's behavior is byte-identical,
    /// and an author who set the bit carelessly is still refused. The permit
    /// exists for tools whose effect is approved once by the run's approved
    /// scope (plan-gated egress, not per-call approval) — the composition
    /// root derives it from the step's approved scopes, and a tool the step
    /// never approved is absent from its definitions regardless, so the
    /// permit alone can admit nothing unapproved.
    pub permit_external_effects: bool,
    /// Ceiling on the approximate byte size of the conversation the loop has
    /// assembled (assistant turns plus tool results grow it past the pre-loop
    /// history budget). Breaching it trims the oldest tool-result groups
    /// rather than sending an ever-growing payload to the provider.
    pub context_byte_budget: usize,
}
impl Default for AgentLimits {
    fn default() -> Self {
        Self {
            max_turns: None,
            max_tool_calls: None,
            max_continuations: Some(super::DEFAULT_MAX_CONTINUATIONS),
            permit_candidate_writes: false,
            permit_workspace_writes: false,
            permit_external_effects: false,
            context_byte_budget: 256 * 1024,
        }
    }
}

/// Environment variable naming the turn ceiling (`SAYA_AGENT_MAX_TURNS`).
pub const MAX_TURNS_ENV: &str = "SAYA_AGENT_MAX_TURNS";
/// Environment variable naming the tool-call ceiling (`SAYA_AGENT_MAX_TOOL_CALLS`).
pub const MAX_TOOL_CALLS_ENV: &str = "SAYA_AGENT_MAX_TOOL_CALLS";
/// Environment variable naming the continuation ceiling
/// (`SAYA_AGENT_MAX_CONTINUATIONS`).
pub const MAX_CONTINUATIONS_ENV: &str = "SAYA_AGENT_MAX_CONTINUATIONS";

/// Default ceiling on continuations after the provider caps a response
/// mid-answer. Bounded by default so an operator who never heard of
/// [`MAX_CONTINUATIONS_ENV`] still gets the bound.
pub const DEFAULT_MAX_CONTINUATIONS: u32 = 3;

/// The turn, tool-call, and continuation ceilings read from the environment.
///
/// Returned by [`budgets_from_env`] so the call site names each ceiling
/// instead of indexing a bare tuple.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnvBudgets {
    pub max_turns: Option<usize>,
    pub max_tool_calls: Option<usize>,
    pub max_continuations: Option<u32>,
}

/// The tool name the model calls to designate the SQL that answers the
/// question. Recognised by the loop at the terminal turn; the SQL text is
/// carried on [`AgentOutput::answer_sql`] and an `AnswerDesignated` event, not
/// executed as a tool.
pub const DESIGNATE_ANSWER_TOOL: &str = "designate_answer";

/// Reads the turn, tool-call, and continuation ceilings from the environment
/// through `get`.
///
/// Turns and tool calls are optional: `None` (unset or unparseable) leaves the
/// loop unbounded for that ceiling, and a set value imposes no upper limit.
/// Continuations differ: unset or unparseable means the default bound
/// (`Some(3)`), never unbounded — an operator who never heard of the variable
/// still gets the bound. `Some(0)` (via `SAYA_AGENT_MAX_CONTINUATIONS=0`)
/// disables continuation entirely. `get` is a callback rather than a direct
/// `std::env::var` so the parsing is testable without mutating process-global
/// environment; the caller supplies the env source.
pub fn budgets_from_env(get: impl Fn(&str) -> Option<String>) -> EnvBudgets {
    EnvBudgets {
        max_turns: parse_budget(get(super::MAX_TURNS_ENV)),
        max_tool_calls: parse_budget(get(super::MAX_TOOL_CALLS_ENV)),
        max_continuations: parse_continuations(get(super::MAX_CONTINUATIONS_ENV)),
    }
}

fn parse_budget(value: Option<String>) -> Option<usize> {
    value.and_then(|raw| raw.trim().parse().ok())
}

fn parse_continuations(value: Option<String>) -> Option<u32> {
    match value {
        None => Some(DEFAULT_MAX_CONTINUATIONS),
        Some(raw) => raw.trim().parse().ok().or(Some(DEFAULT_MAX_CONTINUATIONS)),
    }
}
// `events` holds `AgentEvent`, which carries a `serde_json::Value` and is
// therefore `PartialEq` but not `Eq`.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentOutput {
    pub answer: String,
    pub events: Vec<AgentEvent>,
    pub used_bounded_sql_query: bool,
    pub tool_metadata: Vec<ToolMetadata>,
    /// Token counts summed over every provider turn of this run (zero when
    /// the provider does not report usage).
    pub usage: crate::TokenUsage,
    /// Token counts the post-turn extraction call reported, when it produced a
    /// response. `None` covers both "no extraction ran" (learning disabled or
    /// the gate declined) and "extraction ran but produced no response"
    /// (provider error or timeout) — absent is not zero, so the recorder must
    /// not fold it in as a row of zeros. Kept separate from `usage` so the
    /// answering total's meaning is unchanged and `/usage` can label the two
    /// calls apart.
    pub learning_usage: Option<crate::TokenUsage>,
    /// `true` when the answer is the best the model could produce after a
    /// budget ran out mid-run, rather than a natural completion. The run did
    /// not finish on its own; the caller may surface this so a reader knows the
    /// answer is bounded-effort, not a clean result.
    pub truncated: bool,
    /// The SQL the model designated as the query that answers the question, or
    /// `None` when the model did not designate one. Optional by design: a turn
    /// that never designates works exactly as before. Carries the SQL text
    /// only — already user-visible via tool-call detail — never result rows.
    pub answer_sql: Option<String>,
}
#[derive(Debug, Error)]
pub enum AgentError {
    #[error("{0}")]
    Provider(#[from] ProviderError),
    #[error("agent limit reached: {0}")]
    Limit(&'static str),
    #[error("provider returned an unsupported tool call")]
    InvalidToolCall,
    #[error("conversation history is invalid")]
    InvalidHistory,
    #[error("request cancelled")]
    Cancelled,
}
