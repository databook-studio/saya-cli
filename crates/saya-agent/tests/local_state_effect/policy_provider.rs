/// A provider that emits all of a turn's tool calls together, then answers on
/// the next request. It exercises the runner's single and multi-call paths at
/// the public agent boundary.
struct PolicyProvider {
    calls: Vec<ToolCall>,
    turn: Mutex<u32>,
}
#[async_trait]
impl ChatProvider for PolicyProvider {
    fn name(&self) -> &str {
        "policy-mock"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, saya_agent::ProviderError> {
        unreachable!("stream path is used")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: saya_agent::CancellationToken,
    ) -> Result<saya_agent::ProviderStream, saya_agent::ProviderError> {
        let first = {
            let mut turn = self.turn.lock().unwrap();
            let was = *turn;
            *turn += 1;
            was == 0
        };
        let events = if first {
            vec![
                Ok(saya_agent::ProviderEvent::ToolCalls(self.calls.clone())),
                Ok(saya_agent::ProviderEvent::Done),
            ]
        } else {
            vec![
                Ok(saya_agent::ProviderEvent::TextDelta("done".into())),
                Ok(saya_agent::ProviderEvent::Done),
            ]
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}
