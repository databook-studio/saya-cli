use super::non_execution_replies;
use crate::{ChatMessage, ToolCall};

#[test]
fn context_refusal_keeps_one_literal_paired_reply_for_every_valid_id() {
    let assistant = ChatMessage {
        role: "assistant".into(),
        content: "partial".into(),
        tool_calls: vec![
            ToolCall {
                id: "call-a".into(),
                name: "query".into(),
                arguments: serde_json::json!({"sql":"SELECT 1"}),
            },
            ToolCall {
                id: "call-b".into(),
                name: "unknown".into(),
                arguments: serde_json::json!({"x":2}),
            },
        ],
        tool_call_id: None,
    };
    let replies = non_execution_replies(&assistant);
    assert_eq!(
        replies
            .iter()
            .map(|message| message.role.as_str())
            .collect::<Vec<_>>(),
        ["tool", "tool"]
    );
    assert_eq!(
        replies
            .iter()
            .map(|message| message.tool_call_id.as_deref())
            .collect::<Vec<_>>(),
        [Some("call-a"), Some("call-b")]
    );
    assert!(
        replies
            .iter()
            .all(|message| { message.content == r#"{"error":"request context limit reached"}"# })
    );
}
