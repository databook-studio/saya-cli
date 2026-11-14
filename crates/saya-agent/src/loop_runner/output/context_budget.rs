use super::compaction_pass::compact;

/// Approximate serialized size of a message, including tool-call arguments,
/// which dominate real growth during multi-turn runs.
pub(in crate::loop_runner) fn message_size(message: &crate::ChatMessage) -> usize {
    message.content.len()
        + message.role.len()
        + message.reasoning.as_deref().map_or(0, str::len)
        + message.reasoning_signature.as_deref().map_or(0, str::len)
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
pub(in crate::loop_runner) fn completion_summary(base: &str, truncated: bool) -> String {
    if truncated {
        format!("{base} (truncated)")
    } else {
        base.to_owned()
    }
}

/// The marker appended to an in-place-truncated tool message so the model
/// knows the result it received was cut.
pub(super) const TRUNCATED_MARKER: &str =
    "…[truncated: tool result exceeded the conversation byte budget]";
/// The share of the configured conversation budget at which history is
/// compacted. The conversation grows freely up to it — there is no separate
/// working-suffix cap and no tool-group cap — so an operator who configured
/// megabytes of context keeps megabytes of history.
const COMPACTION_TRIGGER_PERCENT: usize = 90;
/// The share of the configured conversation budget one compaction drains to.
/// Draining well below the trigger buys many append-only turns before the
/// trigger is crossed again. The prompt cache is a prefix cache and a
/// compaction rewrites the suffix behind the cached prefix, so a deeper drain
/// is a cache miss once per compaction rather than once per turn.
const COMPACTION_DRAIN_PERCENT: usize = 60;

/// The two ceilings that bound one conversation. The byte budget is the hard
/// ceiling; when the model window is known, the token trigger is the earlier
/// point at which a compaction runs.
#[derive(Debug, Clone, Copy)]
pub(in crate::loop_runner) struct ContextBudget {
    /// Hard ceiling on the approximate serialized size of the conversation.
    pub(in crate::loop_runner) bytes: usize,
    /// Token count at which compaction triggers, from the known context
    /// window. `None` leaves the byte trigger in charge.
    pub(in crate::loop_runner) token_trigger: Option<u64>,
}

impl ContextBudget {
    pub(in crate::loop_runner) fn new(bytes: usize, token_trigger: Option<u64>) -> Self {
        Self {
            bytes,
            token_trigger,
        }
    }
}

/// Enforces the intra-loop conversation budget in place, never aborting the
/// run. The conversation grows freely until it crosses the earlier of two
/// marks — the token trigger derived from a known context window, or
/// [`COMPACTION_TRIGGER_PERCENT`] of the byte budget — and below that mark it
/// is left untouched. Past the mark a single compaction evicts the oldest
/// tool-result groups and replaces them with one short digest (the statements
/// that ran and the shapes of their results — never row values), keeping the
/// system message and the most recent groups intact, and drains the
/// conversation well below the mark. Draining well below the trigger means the
/// loop then appends for many turns before a compaction runs again — one
/// compaction per trigger, never a per-turn trickle — and the protected prefix
/// is byte-identical across compactions, so the provider's cached prefix keeps
/// hitting.
///
/// An already-sent tool message is never rewritten: only the newest group —
/// the one part of the conversation not yet sent to the provider at the point
/// of trimming — may be truncated, and only when it alone still exceeds the
/// byte budget.
pub(in crate::loop_runner) fn trim_to_budget(
    messages: &mut Vec<crate::ChatMessage>,
    budget: ContextBudget,
    protected_prefix_len: usize,
    sink: Option<&dyn crate::AgentEventSink>,
    checkpoint: &super::super::coding_checkpoint::CodingCheckpoint,
) {
    let total = messages.iter().map(super::message_size).sum::<usize>();
    if total <= trigger_bytes(budget) {
        return;
    }
    let drain = drain_bytes(budget);
    let protected = messages[..protected_prefix_len.min(messages.len())].to_vec();
    let (evicted, truncated) = compact(
        messages,
        (budget.bytes, drain),
        protected_prefix_len,
        checkpoint,
    );
    if (evicted > 0 || truncated > 0)
        && let Some(sink) = sink
    {
        sink.observe_compaction(crate::ContextCompaction {
            before_bytes: total as u64,
            after_bytes: messages.iter().map(super::message_size).sum::<usize>() as u64,
            evicted_tool_groups: evicted as u64,
            truncated_tool_messages: truncated as u64,
            protected_prefix_messages: protected_prefix_len as u64,
            protected_prefix_unchanged: messages.get(..protected.len())
                == Some(protected.as_slice()),
        });
    }
}

/// Bytes at which a compaction triggers: the earlier of the token trigger
/// derived from a known window and the byte budget's own trigger. The byte
/// budget stays a hard ceiling, so a large window can never raise it.
fn trigger_bytes(budget: ContextBudget) -> usize {
    let byte_trigger = percent_of(budget.bytes, COMPACTION_TRIGGER_PERCENT);
    match budget.token_trigger {
        Some(tokens) => bytes_for_tokens(tokens).min(byte_trigger),
        None => byte_trigger,
    }
}

/// Bytes a compaction drains to: the same share below the trigger as the byte
/// budget uses, so a token-triggered compaction leaves the same append-only
/// room before the next one.
fn drain_bytes(budget: ContextBudget) -> usize {
    let byte_drain = percent_of(budget.bytes, COMPACTION_DRAIN_PERCENT);
    match budget.token_trigger {
        Some(tokens) => byte_drain.min(
            bytes_for_tokens(tokens).saturating_mul(COMPACTION_DRAIN_PERCENT)
                / COMPACTION_TRIGGER_PERCENT,
        ),
        None => byte_drain,
    }
}

/// The byte equivalent of a token count, using the same bytes-per-token
/// estimate the stream cut reports use.
fn bytes_for_tokens(tokens: u64) -> usize {
    let bytes = (tokens as f64 * crate::providers::framing::ESTIMATE_BYTES_PER_TOKEN).ceil();
    bytes.min(usize::MAX as f64) as usize
}

/// `budget` scaled by `percent`, saturating rather than overflowing.
fn percent_of(budget: usize, percent: usize) -> usize {
    budget.saturating_mul(percent) / 100
}
