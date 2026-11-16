/// A provider whose stream reports a content-free transport failure after the
/// stream was established. It is the loop-level reading of a reset connection,
/// so the turn retries it on the existing schedule.
struct AlwaysFailingProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}
#[async_trait]
impl ChatProvider for AlwaysFailingProvider {
    fn name(&self) -> &str {
        "always-failing"
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
        Ok(Box::pin(futures_util::stream::iter(vec![Err(
            saya_agent::ProviderError::Request("network request failed".into()),
        )])))
    }
}
