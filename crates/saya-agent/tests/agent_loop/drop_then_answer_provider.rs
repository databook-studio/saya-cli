/// A provider whose first stream drops mid-answer (partial deltas, the stream
/// ends without `Done`) and whose retry succeeds. Drives the M1-7 contract:
/// the run completes, the sink saw reset-then-full text and never duplicated
/// text, and the retry re-sent the conversation exactly as it stood at turn
/// start.
struct DropThenAnswerProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}
#[async_trait]
impl ChatProvider for DropThenAnswerProvider {
    fn name(&self) -> &str {
        "drop-then-answer"
    }
    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, saya_agent::ProviderError> {
        unreachable!("the main loop drives the provider through stream")
    }
    async fn stream(
        &self,
        request: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, saya_agent::ProviderError> {
        let attempt = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request);
            requests.len() - 1
        };
        let events = if attempt == 0 {
            // The stream drops mid-answer: partial deltas, no `Done`.
            vec![
                Ok(ProviderEvent::TextDelta("The an".into())),
                Ok(ProviderEvent::TextDelta("sw".into())),
            ]
        } else {
            vec![
                Ok(ProviderEvent::TextDelta("The answer is 42.".into())),
                Ok(ProviderEvent::Done),
            ]
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}
