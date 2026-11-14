struct OptionalUsageRetry {
    calls: Mutex<usize>,
    failed_usage: Option<TokenUsage>,
    successful_usage: Option<TokenUsage>,
}
#[async_trait]
impl ChatProvider for OptionalUsageRetry {
    fn name(&self) -> &str {
        "optional-usage-retry"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the agent receive path streams")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        let attempt = *calls;
        let usage = if attempt == 1 {
            self.failed_usage
        } else {
            self.successful_usage
        };
        // The failed attempt delivers no content on purpose: it has to stay a
        // retryable `StreamEnded`, and a stream end after content is a cut.
        let mut events = if attempt == 1 {
            Vec::new()
        } else {
            vec![Ok(ProviderEvent::TextDelta("kept".into()))]
        };
        if let Some(usage) = usage {
            events.push(Ok(ProviderEvent::Usage(usage)));
        }
        if attempt > 1 {
            events.push(Ok(ProviderEvent::Done));
        }
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}
