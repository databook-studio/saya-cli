use crate::history_context::render_context;
use crate::{AgentError, ChatMessage, ContextBlock};

/// Fixed system prefix: identity and posture, deliberately capability-silent.
///
/// The tool list (schemas) and the approval engine decide what is possible
/// this turn — not prose. Any sentence naming or describing capabilities here
/// drifts false the moment a tool is added, so this prefix only names SAYA,
/// defers to the tools actually given, and keeps the honesty rule. Do not add
/// capability claims back.
pub(super) const SYSTEM_PROMPT: &str =
    "You are SAYA. Act through the tools you were given. Never claim an action you did not take.";

/// Builds the message list for the agent from optional extra system prompt context,
/// untrusted context blocks, the user prompt, and conversation history.
///
/// History is bounded by bytes, not by message count: the replay loop keeps
/// the newest user/assistant pairs that fit the byte budget left after the
/// system and user turns. A count cap and a byte cap are two mechanisms for
/// one job, and the byte cap is the one that tracks what the provider
/// actually charges for — so there is no count cap here. A prompt that alone
/// fills the budget simply gets no history rather than failing the run. The
/// loop bounds the conversation during execution with its
/// own `context_byte_budget` (the same value passed here), so the start-of-run
/// bound and the in-loop bound agree.
///
/// Context blocks are rendered into the **user** turn (never the system message) as
/// quoted, labelled data; see [`render_context`].
pub fn build_messages(
    system_extra: Option<&str>,
    context_blocks: &[ContextBlock],
    prompt: &str,
    history: &[ChatMessage],
    byte_budget: usize,
) -> Result<Vec<ChatMessage>, AgentError> {
    build_messages_with_history_count(system_extra, context_blocks, prompt, history, byte_budget)
        .map(|(messages, _, _)| messages)
}

/// Builds the same request as [`build_messages`] and returns how many complete
/// replay-history pairs were selected and the boundary after the protected
/// system, selected history, and original current-user messages.
pub(crate) fn build_messages_with_history_count(
    system_extra: Option<&str>,
    context_blocks: &[ContextBlock],
    prompt: &str,
    history: &[ChatMessage],
    byte_budget: usize,
) -> Result<(Vec<ChatMessage>, usize, usize), AgentError> {
    let system_content = system_content(system_extra);
    let user_content = render_context(context_blocks, prompt);
    let current = [
        ChatMessage::text("system", system_content),
        ChatMessage::text("user", user_content),
    ];
    let current_bytes = current.iter().map(message_bytes).sum::<usize>();
    validate(history)?;
    let budget = byte_budget.saturating_sub(current_bytes);
    let mut chosen = Vec::new();
    let mut history_bytes = 0;
    for pair in history.as_chunks::<2>().0.iter().rev() {
        let pair_bytes = pair.iter().map(message_bytes).sum::<usize>();
        if history_bytes + pair_bytes > budget {
            break;
        }
        history_bytes += pair_bytes;
        chosen.push(pair.to_vec());
    }
    chosen.reverse();
    let history_pair_count = chosen.len();
    let mut messages = vec![current[0].clone()];
    messages.extend(chosen.into_iter().flatten());
    messages.push(current[1].clone());
    let protected_prefix_len = messages.len();
    Ok((messages, history_pair_count, protected_prefix_len))
}

/// Removes the oldest selected replay pair without touching the fixed system
/// or current-user messages, updating the assembled boundary with that pair.
pub(crate) fn discard_oldest_history_pair(
    messages: &mut Vec<ChatMessage>,
    pair_count: &mut usize,
    protected_prefix_len: &mut usize,
) -> bool {
    if *pair_count == 0 || *protected_prefix_len < 3 {
        return false;
    }
    messages.drain(1..3);
    *pair_count -= 1;
    *protected_prefix_len -= 2;
    true
}

/// The system message content [`build_messages`] would assemble from optional
/// extra context: the fixed [`SYSTEM_PROMPT`], plus the extra when it is non-empty.
/// Shared with [`turn_bytes`] so the pre-build budget check and the post-build
/// message use one system-message shape and cannot drift.
fn system_content(extra: Option<&str>) -> String {
    match extra {
        Some(s) if !s.trim().is_empty() => format!("{SYSTEM_PROMPT}\n\n{s}"),
        _ => SYSTEM_PROMPT.to_string(),
    }
}

/// The exact byte size of the system + user messages [`build_messages`] would
/// assemble from `system`, `blocks`, and `prompt` (no history): the post-escape,
/// post-wrapper content the agent will actually send. Exposed so the
/// prompt-recall path can bound a context block body to what fits *before* the
/// request is built, using the same accounting [`build_messages`] uses, so the
/// two cannot drift. The block's `truncated` flag changes the wrapper size, so a
/// caller reserving the worst case passes `truncated: true`.
pub fn turn_bytes(system: Option<&str>, blocks: &[ContextBlock], prompt: &str) -> usize {
    let system_msg = system_content(system);
    let user_content = render_context(blocks, prompt);
    message_bytes(&ChatMessage::text("system", system_msg))
        + message_bytes(&ChatMessage::text("user", user_content))
}

fn validate(history: &[ChatMessage]) -> Result<(), AgentError> {
    if !history.len().is_multiple_of(2) {
        return Err(AgentError::InvalidHistory);
    }
    for (index, message) in history.iter().enumerate() {
        let expected = if index % 2 == 0 { "user" } else { "assistant" };
        if message.role != expected
            || !message.tool_calls.is_empty()
            || message.tool_call_id.is_some()
        {
            return Err(AgentError::InvalidHistory);
        }
    }
    Ok(())
}

pub(super) fn message_bytes(message: &ChatMessage) -> usize {
    message.role.len() + message.content.len()
}
