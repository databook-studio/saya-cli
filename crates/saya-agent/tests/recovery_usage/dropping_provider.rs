struct DroppingProvider {
    calls: Arc<Mutex<usize>>,
}
#[async_trait]
impl ChatProvider for DroppingProvider {
    fn name(&self) -> &str {
        "dropping"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the agent receive path streams")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        *self.calls.lock().unwrap() += 1;
        Ok(Box::pin(futures_util::stream::empty()))
    }
}
