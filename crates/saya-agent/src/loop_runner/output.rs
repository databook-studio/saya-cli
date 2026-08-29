use crate::{AgentEvent, ProviderError, ToolMetadata};
use thiserror::Error;

#[derive(Debug, Clone, Copy)]
pub struct AgentLimits {
    pub max_turns: usize,
    pub max_tool_calls: usize,
    /// Whether the loop may execute tools that declare
    /// [`LocalStateEffect::WriteCandidate`](crate::LocalStateEffect::WriteCandidate).
    /// Defaults to **not permitted**: a tool that can write a candidate claim
    /// must not start writing merely because it was registered. Phase 4 turns
    /// this on under an explicit config setting; until then nothing can enable
    /// it, which is correct.
    pub permit_candidate_writes: bool,
    /// Ceiling on the approximate byte size of the conversation the loop has
    /// assembled (assistant turns plus tool results grow it past the pre-loop
    /// history budget). Breaching it fails closed instead of sending an ever
    /// growing payload to the provider.
    pub context_byte_budget: usize,
}
impl Default for AgentLimits {
    fn default() -> Self {
        Self {
            max_turns: 12,
            max_tool_calls: 24,
            permit_candidate_writes: false,
            context_byte_budget: 256 * 1024,
        }
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
    #[error("conversation context exceeds the safe limit")]
    ContextLimit,
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
