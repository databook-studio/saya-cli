/// A scripted stream provider for the continuation tests. The first turn (or
/// every turn when `always_truncate`) streams the partial text and then caps
/// mid-answer with `OutputTruncated`; later turns answer with fixed prose.
/// Every request is recorded so tests can inspect what the provider saw.
struct ContinuationScriptProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
    calls: Mutex<usize>,
    always_truncate: bool,
    partial_text: String,
    partial_tool_json: Vec<String>,
}
#[async_trait]
impl ChatProvider for ContinuationScriptProvider {
    fn name(&self) -> &str {
        "continuation-script"
    }
    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, saya_agent::ProviderError> {
        unreachable!("the main loop drives the provider through stream")
    }
    async fn stream(
        &self,
        request: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, saya_agent::ProviderError> {
        self.requests.lock().unwrap().push(request);
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        let turn = *calls;
        drop(calls);
        if !self.always_truncate && turn > 1 {
            return Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(ProviderEvent::TextDelta("cooperative answer".into())),
                Ok(ProviderEvent::Done),
            ])));
        }
        Ok(Box::pin(futures_util::stream::iter(vec![
            Ok(ProviderEvent::TextDelta(self.partial_text.clone())),
            Err(saya_agent::ProviderError::output_truncated(
                self.partial_text.clone(),
                self.partial_tool_json.clone(),
            )),
        ])))
    }
}
