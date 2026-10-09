struct MaxThenSuccess {
    attempts: Mutex<usize>,
}
#[async_trait]
impl ChatProvider for MaxThenSuccess {
    fn name(&self) -> &str {
        "max-then-success"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the agent receive path streams")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        let attempt = {
            let mut attempts = self.attempts.lock().unwrap();
            *attempts += 1;
            *attempts
        };
        let events = if attempt == 1 {
            vec![Ok(ProviderEvent::Usage(
                TokenUsage::new(u64::MAX, u64::MAX)
                    .with_cached_input(Some(u64::MAX))
                    .with_cache_creation(Some(u64::MAX))
                    .with_reasoning(Some(u64::MAX)),
            ))]
        } else {
            vec![
                Ok(ProviderEvent::TextDelta("done".into())),
                Ok(ProviderEvent::Usage(
                    TokenUsage::new(1, 1)
                        .with_cached_input(Some(1))
                        .with_cache_creation(Some(1))
                        .with_reasoning(Some(1)),
                )),
                Ok(ProviderEvent::Done),
            ]
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}
