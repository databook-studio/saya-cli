/// A provider that records every request, answers the first call with one
/// `schema_discovery` tool call, and answers the next with a plain final text.
struct ToolThenAnswerProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}
#[async_trait]
impl ChatProvider for ToolThenAnswerProvider {
    fn name(&self) -> &str {
        "tool-then-answer"
    }
    async fn complete(
        &self,
        request: ChatRequest,
    ) -> Result<ChatResponse, saya_agent::ProviderError> {
        let first = self.requests.lock().unwrap().is_empty();
        self.requests.lock().unwrap().push(request);
        if first {
            Ok(ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "call-1".into(),
                    name: "schema_discovery".into(),
                    arguments: serde_json::json!({}),
                }],
                tool_call_id: None,
            }))
        } else {
            Ok(ChatResponse::new(ChatMessage::text("assistant", "done")))
        }
    }
}
