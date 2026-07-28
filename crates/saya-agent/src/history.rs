use crate::{AgentError, ChatMessage};

pub const MAX_HISTORY_MESSAGES: usize = 20;
pub const MAX_HISTORY_BYTES: usize = 32 * 1024;
const SYSTEM_PROMPT: &str = "You are SAYA, a database assistant. Use only the supplied read-only tools. Never claim to have written data or used unsupported tools.";

pub fn build_messages(
    prompt: &str,
    history: &[ChatMessage],
) -> Result<Vec<ChatMessage>, AgentError> {
    let current = [
        ChatMessage::text("system", SYSTEM_PROMPT),
        ChatMessage::text("user", prompt),
    ];
    let current_bytes = current.iter().map(message_bytes).sum::<usize>();
    if current_bytes > MAX_HISTORY_BYTES {
        return Err(AgentError::ContextLimit);
    }
    validate(history)?;
    let budget = MAX_HISTORY_BYTES - current_bytes;
    let mut chosen = Vec::new();
    let mut selected_messages = 0;
    let mut history_bytes = 0;
    for pair in history.chunks_exact(2).rev() {
        let pair_bytes = pair.iter().map(message_bytes).sum::<usize>();
        if selected_messages + pair.len() > MAX_HISTORY_MESSAGES
            || history_bytes + pair_bytes > budget
        {
            break;
        }
        selected_messages += pair.len();
        history_bytes += pair_bytes;
        chosen.push(pair.to_vec());
    }
    chosen.reverse();
    let mut messages = vec![current[0].clone()];
    messages.extend(chosen.into_iter().flatten());
    messages.push(current[1].clone());
    Ok(messages)
}

fn validate(history: &[ChatMessage]) -> Result<(), AgentError> {
    if history.len() % 2 != 0 {
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

fn message_bytes(message: &ChatMessage) -> usize {
    message.role.len() + message.content.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_newest_complete_turns_with_stable_bounds() {
        let history = (0..24)
            .flat_map(|index| {
                [
                    ChatMessage::text("user", format!("u{index}")),
                    ChatMessage::text("assistant", format!("a{index}")),
                ]
            })
            .collect::<Vec<_>>();
        let messages = build_messages("current", &history).unwrap();
        assert_eq!(messages.len(), 22);
        assert_eq!(messages[1].content, "u14");
        assert_eq!(messages[20].content, "a23");
        assert_eq!(messages[1].role, "user");
        assert_eq!(messages[2].role, "assistant");
        assert_eq!(messages[3].role, "user");
        assert!(messages.iter().map(message_bytes).sum::<usize>() <= MAX_HISTORY_BYTES);
    }

    #[test]
    fn rejects_tool_history_and_oversized_current_prompt() {
        let history = vec![ChatMessage {
            role: "tool".into(),
            content: "row-sentinel".into(),
            tool_calls: Vec::new(),
            tool_call_id: Some("call".into()),
        }];
        assert!(matches!(
            build_messages("ok", &history),
            Err(AgentError::InvalidHistory)
        ));
        assert!(matches!(
            build_messages(&"x".repeat(MAX_HISTORY_BYTES), &[]),
            Err(AgentError::ContextLimit)
        ));
    }

    #[test]
    fn cumulative_byte_boundary_keeps_only_newest_contiguous_suffix() {
        let large = "x".repeat(MAX_HISTORY_BYTES / 2);
        let history = vec![
            ChatMessage::text("user", "old"),
            ChatMessage::text("assistant", "old-answer"),
            ChatMessage::text("user", large.clone()),
            ChatMessage::text("assistant", large),
        ];
        let messages = build_messages("current", &history).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].content, "current");
    }
}
