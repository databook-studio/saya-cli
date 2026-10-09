struct LatestUsageThenSuccess {
    attempts: Mutex<usize>,
}
#[async_trait]
impl ChatProvider for LatestUsageThenSuccess {
    fn name(&self) -> &str {
        "latest-usage-then-success"
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
            vec![
                Ok(ProviderEvent::TextDelta("discarded".into())),
                Ok(ProviderEvent::Usage(TokenUsage::new(3, 1))),
                Ok(ProviderEvent::Usage(TokenUsage::new(7, 2))),
            ]
        } else {
            vec![
                Ok(ProviderEvent::TextDelta("kept".into())),
                Ok(ProviderEvent::Usage(TokenUsage::new(11, 5))),
                Ok(ProviderEvent::Done),
            ]
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}
