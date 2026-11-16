/// Retries are bounded: a provider that drops every attempt exhausts the
/// backoff schedule (one attempt plus the eight scheduled sleeps) and then
/// falls through to the existing error path, never looping forever. The drop
/// delivers nothing: a drop that had already delivered content is a stream cut
/// and takes the continuation path instead, which is what
/// `a_cut_after_content_is_typed_and_not_resent` pins.
struct AlwaysDroppingProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}
#[async_trait]
impl ChatProvider for AlwaysDroppingProvider {
    fn name(&self) -> &str {
        "always-dropping"
    }
    fn retry_delays(&self) -> Vec<std::time::Duration> {
        vec![std::time::Duration::from_millis(1); 8]
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
        Ok(Box::pin(futures_util::stream::empty()))
    }
}
