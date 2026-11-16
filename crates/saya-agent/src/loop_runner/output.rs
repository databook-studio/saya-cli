#[path = "output/compaction_pass.rs"]
mod compaction_pass;
#[path = "output/context_budget.rs"]
mod context_budget;
#[path = "output/group_trim.rs"]
mod group_trim;
#[path = "output/limits.rs"]
mod limits;
#[path = "output/trim_envelope.rs"]
mod trim_envelope;

#[cfg(test)]
use context_budget::TRUNCATED_MARKER;
pub(super) use context_budget::message_size;
pub(super) use context_budget::{ContextBudget, completion_summary, trim_to_budget};
pub use limits::budgets_from_env;
pub use limits::{AgentError, AgentLimits, AgentOutput, DESIGNATE_ANSWER_TOOL, EnvBudgets};
pub use limits::{
    DEFAULT_MAX_CONTINUATIONS, MAX_CONTINUATIONS_ENV, MAX_TOOL_CALLS_ENV, MAX_TURNS_ENV,
};

#[cfg(test)]
#[path = "output/tests.rs"]
mod tests;
