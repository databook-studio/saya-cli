/// O1 property 5: a truncation is never retried — exactly one attempt, and no
/// `TurnReset` (the renderer prints "interrupted — retrying" per reset). The
/// cap is deterministic, so re-sending the identical request is pure waste.
struct TruncatingProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}
#[async_trait]
impl ChatProvider for TruncatingProvider {
    fn name(&self) -> &str {
        "truncating"
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
            Ok(ProviderEvent::TextDelta("partial".into())),
            Err(saya_agent::ProviderError::output_truncated(
                "partial".into(),
                Vec::new(),
            )),
        ])))
    }
}
