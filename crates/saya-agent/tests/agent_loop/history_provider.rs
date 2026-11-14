struct HistoryProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}
#[async_trait]
impl ChatProvider for HistoryProvider {
    fn name(&self) -> &str {
        "history-mock"
    }

    async fn complete(
        &self,
        request: ChatRequest,
    ) -> Result<ChatResponse, saya_agent::ProviderError> {
        self.requests.lock().unwrap().push(request);
        Ok(ChatResponse::new(ChatMessage::text(
            "assistant",
            "second answer",
        )))
    }
}
