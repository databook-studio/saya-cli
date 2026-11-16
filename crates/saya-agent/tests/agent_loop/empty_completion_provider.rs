/// A stream that **completes** (`Done`) but carries no usable response is not
/// a mid-stream failure: the model answered nothing, and every retry would
/// fail the same way, so it errors immediately after the one attempt instead
/// of burning the backoff schedule.
struct EmptyCompletionProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}
#[async_trait]
impl ChatProvider for EmptyCompletionProvider {
    fn name(&self) -> &str {
        "empty-completion"
    }
    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, saya_agent::ProviderError> {
        unreachable!("the main loop drives the provider through stream")
    }
    async fn stream(
        &self,
        request: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, saya_agent::ProviderError> {
        self.requests.lock().unwrap().push(request);
        Ok(Box::pin(futures_util::stream::iter(vec![
            Ok(ProviderEvent::TextDelta(String::new())),
            Ok(ProviderEvent::Done),
        ])))
    }
}
