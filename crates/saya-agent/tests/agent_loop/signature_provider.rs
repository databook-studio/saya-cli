/// A provider that reports one signed thinking block with a tool call, then
/// answers. The loop must carry the signature onto the assistant message it
/// stores, so the next request can replay the signed block instead of
/// re-deriving the thinking.
struct SignatureProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl ChatProvider for SignatureProvider {
    fn name(&self) -> &str {
        "signature"
    }
    async fn complete(
        &self,
        _: ChatRequest,
    ) -> Result<ChatResponse, saya_agent::ProviderError> {
        unreachable!("the main loop drives the provider through stream")
    }
    async fn stream(
        &self,
        request: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, saya_agent::ProviderError> {
        self.requests.lock().unwrap().push(request);
        if self
            .calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            == 0
        {
            return Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(ProviderEvent::ReasoningDelta("plan".into())),
                Ok(ProviderEvent::ReasoningSignature("sig-1".into())),
                Ok(ProviderEvent::ToolCalls(vec![ToolCall {
                    id: "call-1".into(),
                    name: "schema_discovery".into(),
                    arguments: serde_json::json!({}),
                }])),
                Ok(ProviderEvent::Done),
            ])));
        }
        Ok(Box::pin(futures_util::stream::iter(vec![
            Ok(ProviderEvent::TextDelta("done".into())),
            Ok(ProviderEvent::Done),
        ])))
    }
}
