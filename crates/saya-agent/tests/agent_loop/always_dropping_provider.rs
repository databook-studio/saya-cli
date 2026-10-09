/// Retries are bounded: a provider that drops every attempt exhausts the
/// backoff schedule (one attempt plus the three scheduled sleeps) and then
/// falls through to the existing error path, never looping forever.
struct AlwaysDroppingProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}
#[async_trait]
impl ChatProvider for AlwaysDroppingProvider {
    fn name(&self) -> &str {
        "always-dropping"
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
        Ok(Box::pin(futures_util::stream::iter(vec![Ok(
            ProviderEvent::TextDelta("partial".into()),
        )])))
    }
}
