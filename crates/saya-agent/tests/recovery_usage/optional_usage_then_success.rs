struct OptionalUsageThenSuccess {
    attempts: Mutex<usize>,
    first_usage: Option<TokenUsage>,
}
#[async_trait]
impl ChatProvider for OptionalUsageThenSuccess {
    fn name(&self) -> &str {
        "optional-usage-then-success"
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
            self.first_usage
                .map(|usage| vec![Ok(ProviderEvent::Usage(usage))])
                .unwrap_or_default()
        } else {
            vec![
                Ok(ProviderEvent::TextDelta("kept".into())),
                Ok(ProviderEvent::Done),
            ]
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}
