/// O1 property 6: a genuine transport failure still retries on the existing
/// schedule — one attempt plus the three scheduled retries — and the sink saw
/// a reset per retry. The truncation fix must not change this path.
struct AlwaysFailingProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}
#[async_trait]
impl ChatProvider for AlwaysFailingProvider {
    fn name(&self) -> &str {
        "always-failing"
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
