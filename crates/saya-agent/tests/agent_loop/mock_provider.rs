struct MockProvider {
    responses: Mutex<Vec<ChatResponse>>,
}
#[async_trait]
impl ChatProvider for MockProvider {
    fn name(&self) -> &str {
        "mock"
    }
    async fn complete(
        &self,
        request: ChatRequest,
    ) -> Result<ChatResponse, saya_agent::ProviderError> {
        assert_eq!(request.model, "mock-model");
        self.responses.lock().unwrap().remove(0).pipe(Ok)
    }
}
