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
    /// Whether the loop may execute tools that declare
    /// [`LocalStateEffect::WriteCandidate`](crate::LocalStateEffect::WriteCandidate).
    /// Defaults to **not permitted**: a tool that can write a candidate claim
    /// must not start writing merely because it was registered.
    pub permit_candidate_writes: bool,
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
            permit_candidate_writes: false,
            context_byte_budget: 256 * 1024,
        }
    }
}

/// Environment variable naming the turn ceiling (`SAYA_AGENT_MAX_TURNS`).
pub const MAX_TURNS_ENV: &str = "SAYA_AGENT_MAX_TURNS";
/// Environment variable naming the tool-call ceiling (`SAYA_AGENT_MAX_TOOL_CALLS`).
pub const MAX_TOOL_CALLS_ENV: &str = "SAYA_AGENT_MAX_TOOL_CALLS";

/// The tool name the model calls to designate the SQL that answers the
/// question. Recognised by the loop at the terminal turn; the SQL text is
/// carried on [`AgentOutput::answer_sql`] and an `AnswerDesignated` event, not
/// executed as a tool.
pub const DESIGNATE_ANSWER_TOOL: &str = "designate_answer";

/// Reads the turn and tool-call ceilings from the environment through `get`.
///
/// Each is optional: `None` (unset or unparseable) leaves the loop unbounded
/// for that ceiling, and a set value imposes no upper limit. `get` is a
/// callback rather than a direct `std::env::var` so the parsing is testable
/// without mutating process-global environment; the caller supplies the env
/// source.
pub fn budgets_from_env(get: impl Fn(&str) -> Option<String>) -> (Option<usize>, Option<usize>) {
    (
        parse_budget(get(MAX_TURNS_ENV)),
        parse_budget(get(MAX_TOOL_CALLS_ENV)),
    )
}

fn parse_budget(value: Option<String>) -> Option<usize> {
    value.and_then(|raw| raw.trim().parse().ok())
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

/// Approximate serialized size of a message, including tool-call arguments,
/// which dominate real growth during multi-turn runs.
pub(super) fn message_size(message: &crate::ChatMessage) -> usize {
    message.content.len()
        + message.role.len()
        + message
            .tool_calls
            .iter()
            .map(|call| {
                call.id.len()
                    + call.name.len()
                    + serde_json::to_string(&call.arguments).map_or(0, |text| text.len())
            })
            .sum::<usize>()
}

/// A completion summary for a tool result, flagged when the result was
/// truncated to fit the conversation budget so the model is told it saw a cut
/// result rather than a complete one — the model must not be misled.
pub(super) fn completion_summary(base: &str, truncated: bool) -> String {
    if truncated {
        format!("{base} (truncated)")
    } else {
        base.to_owned()
    }
}

/// The marker appended to an in-place-truncated tool message so the model
/// knows the result it received was cut.
const TRUNCATED_MARKER: &str = "…[truncated: tool result exceeded the conversation byte budget]";

/// Enforces the intra-loop conversation byte budget in place, never aborting
/// the run. While the assembled messages exceed `budget`, drops the oldest
/// complete tool-result group — the assistant turn that issued the calls plus
/// its consecutive `tool` messages — so the newest context is retained (the
/// same recency policy the pre-loop history trim uses). When only the newest
/// group remains and it alone is over budget, truncates the largest `tool`
/// message to fit with a visible marker rather than discarding the freshest
/// result or killing the run.
pub(super) fn trim_to_budget(messages: &mut Vec<crate::ChatMessage>, budget: usize) {
    loop {
        let total = messages.iter().map(message_size).sum::<usize>();
        if total <= budget {
            return;
        }
        let groups = tool_groups(messages);
        // Drop the oldest group only when a newer one remains; otherwise the
        // freshest result is kept and truncated below instead of deleted.
        if groups.len() >= 2 {
            let oldest = groups[0].clone();
            messages.drain(oldest);
            continue;
        }
        // No droppable group left — truncate the largest tool message so that,
        // after appending the marker, the whole conversation fits. Done in one
        // step from the current content length so the loop makes progress and
        // terminates.
        let Some(idx) = messages
            .iter()
            .enumerate()
            .filter(|(_, message)| message.role == "tool")
            .max_by_key(|(_, message)| message.content.len())
            .map(|(index, _)| index)
        else {
            // Over budget with no tool message to trim (e.g. a giant system
            // prompt): nothing safe to cut here. Leave it — the pre-loop bound
            // already guards the initial messages.
            return;
        };
        let content_len = messages[idx].content.len();
        // Bytes outside this message's content that count toward the total.
        let overhead = total - content_len;
        // Target content length so content + marker + overhead == budget.
        let target = budget.saturating_sub(overhead + TRUNCATED_MARKER.len());
        if target >= content_len {
            // Truncation cannot help (the marker alone would not fit, or the
            // message is already as small as useful); stop to avoid looping.
            return;
        }
        let bound = super::tools::floor_boundary(&messages[idx].content, target);
        messages[idx].content.truncate(bound);
        messages[idx].content.push_str(TRUNCATED_MARKER);
    }
}

/// Returns the index ranges of complete tool-result groups in `messages`: each
/// is an assistant turn carrying tool calls followed by its consecutive `tool`
/// messages. Ranges are half-open `[start, end)`, suitable for `Vec::drain`.
fn tool_groups(messages: &[crate::ChatMessage]) -> Vec<std::ops::Range<usize>> {
    let mut groups = Vec::new();
    let mut i = 0;
    while i < messages.len() {
        if !messages[i].tool_calls.is_empty() {
            let start = i;
            i += 1;
            while i < messages.len() && messages[i].role == "tool" {
                i += 1;
            }
            groups.push(start..i);
        } else {
            i += 1;
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_limits_are_unbounded() {
        let limits = AgentLimits::default();
        assert!(limits.max_turns.is_none(), "no turn ceiling by default");
        assert!(
            limits.max_tool_calls.is_none(),
            "no tool-call ceiling by default"
        );
    }

    #[test]
    fn budgets_from_env_sets_each_ceiling_independently_with_no_upper_limit() {
        let lookup = |name: &str| match name {
            "SAYA_AGENT_MAX_TURNS" => Some("1000000".to_string()),
            "SAYA_AGENT_MAX_TOOL_CALLS" => Some("7".to_string()),
            _ => None,
        };
        let (turns, tool_calls) = budgets_from_env(lookup);
        assert_eq!(turns, Some(1_000_000));
        assert_eq!(tool_calls, Some(7));
    }

    #[test]
    fn budgets_from_env_is_unbounded_when_unset_or_unparseable() {
        let (turns, tool_calls) = budgets_from_env(|_| None);
        assert!(turns.is_none() && tool_calls.is_none());
        // A set but unparseable value is treated as unset, not as zero: a typo
        // cannot silently collapse the ceiling to the smallest bound.
        let lookup = |name: &str| match name {
            "SAYA_AGENT_MAX_TURNS" => Some("not-a-number".to_string()),
            _ => None,
        };
        let (turns, _) = budgets_from_env(lookup);
        assert!(turns.is_none());
    }
}
