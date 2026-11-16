struct StreamingToolProvider;
#[async_trait]
impl ChatProvider for StreamingToolProvider {
    fn name(&self) -> &str {
        "streaming-mock"
    }
    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, saya_agent::ProviderError> {
        unreachable!()
    }
    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, saya_agent::ProviderError> {
        let call = ToolCall {
            id: "call-1".into(),
            name: "bounded_sql_query".into(),
            arguments: serde_json::json!({"sql":"select 1"}),
        };
        Ok(Box::pin(futures_util::stream::iter(vec![
            Ok(ProviderEvent::ToolCalls(vec![call])),
            Ok(ProviderEvent::Done),
        ])))
    }
}
