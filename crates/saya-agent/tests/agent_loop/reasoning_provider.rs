// --- reasoning crosses the crate boundary, but is not displayed --------

/// A streaming provider whose `stream()` emits a fixed list of events for the
/// one turn the loop drives, so tests can observe exactly which
/// `AgentEvent`s reasoning (or its absence) produces.
struct ReasoningProvider {
    events: Vec<ProviderEvent>,
}
#[async_trait::async_trait]
impl ChatProvider for ReasoningProvider {
    fn name(&self) -> &str {
        "reasoning-mock"
    }
    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, saya_agent::ProviderError> {
        panic!("the loop drives providers through stream()")
    }
    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, saya_agent::ProviderError> {
        Ok(Box::pin(futures_util::stream::iter(
            self.events
                .clone()
                .into_iter()
                .map(Ok::<ProviderEvent, saya_agent::ProviderError>),
        )))
    }
}
