struct ByteLimitThenSuccess {
    attempts: Mutex<usize>,
}
#[async_trait]
impl ChatProvider for ByteLimitThenSuccess {
    fn name(&self) -> &str {
        "byte-limit-then-success"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the agent receive path streams")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        let mut attempts = self.attempts.lock().unwrap();
        *attempts += 1;
        let events = if *attempts == 1 {
            vec![Ok(ProviderEvent::TextDelta(
                "x".repeat(saya_agent::MAX_STREAM_BYTES + 1),
            ))]
        } else {
            vec![
                Ok(ProviderEvent::TextDelta("kept".into())),
                Ok(ProviderEvent::Done),
            ]
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}
