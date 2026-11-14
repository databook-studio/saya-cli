/// A provider whose first stream stalls mid-answer (partial deltas, then the
/// idle-stall error the stream bound raises) and whose retry succeeds. Drives
/// the contract: the run completes, the sink saw reset-then-full text and
/// never duplicated text, and the retry re-sent the conversation exactly as it
/// stood at turn start. A stall is the mid-answer failure that still retries —
/// a stream that *ends* after content is a cut, and continues instead.
struct StallThenAnswerProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}
#[async_trait]
impl ChatProvider for StallThenAnswerProvider {
    fn name(&self) -> &str {
        "stall-then-answer"
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
            // The stream stalls mid-answer: partial deltas, then the idle-stall
            // error, and no `Done`.
            vec![
                Ok(ProviderEvent::TextDelta("The an".into())),
                Ok(ProviderEvent::TextDelta("sw".into())),
                Err(saya_agent::ProviderError::Request(
                    "provider stream stalled".into(),
                )),
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
