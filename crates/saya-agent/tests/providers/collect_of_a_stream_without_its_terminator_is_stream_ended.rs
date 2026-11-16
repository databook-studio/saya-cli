/// A provider whose stream replays a fixed event list, so the default
/// `collect()` is exercised without a network.
struct FixedStreamProvider(Vec<ProviderEvent>);

#[async_trait::async_trait]
impl ChatProvider for FixedStreamProvider {
    fn name(&self) -> &str {
        "fixed-stream"
    }
    async fn complete(
        &self,
        request: ChatRequest,
    ) -> Result<saya_agent::ChatResponse, ProviderError> {
        self.collect(request).await
    }
    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<saya_agent::ProviderStream, ProviderError> {
        Ok(Box::pin(futures_util::stream::iter(
            self.0.clone().into_iter().map(Ok),
        )))
    }
}

#[tokio::test]
async fn collect_of_a_stream_that_ends_without_done_is_stream_ended() {
    let provider = FixedStreamProvider(vec![ProviderEvent::TextDelta("partial".into())]);
    assert!(matches!(
        provider.collect(request()).await,
        Err(ProviderError::StreamEnded)
    ));
}

#[tokio::test]
async fn collect_of_a_completed_stream_with_no_content_stays_invalid() {
    let provider = FixedStreamProvider(vec![ProviderEvent::Done]);
    assert!(matches!(
        provider.collect(request()).await,
        Err(ProviderError::InvalidResponse)
    ));
}
