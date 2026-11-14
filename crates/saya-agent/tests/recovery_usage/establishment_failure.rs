struct EstablishmentFailure {
    calls: Arc<Mutex<usize>>,
}
#[async_trait]
impl ChatProvider for EstablishmentFailure {
    fn name(&self) -> &str {
        "establishment-failure"
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
        Err(ProviderError::Request("request refused".into()))
    }
}
