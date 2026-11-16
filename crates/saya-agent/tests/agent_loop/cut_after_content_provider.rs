// --- a cut after content is typed, and never re-sent identically ---------

/// The reasoning flow a cut attempt had delivered before the gateway closed
/// the stream: 40 deltas of 350 bytes each. Tokens are estimated from those
/// bytes at 3.5 bytes/token, so `ceil(40 * 350 / 3.5) == 4000`.
const CUT_REASONING_DELTAS: usize = 40;
const CUT_DELTA_BYTES: usize = 350;
const CUT_ESTIMATED_TOKENS: u32 = 4000;

/// A provider whose every attempt streams `deltas` reasoning deltas and then
/// reports the stream ended before its terminator — the observed cut, with
/// no usage reported for the cut attempt. Nothing about the request changes
/// between attempts, so a loop that re-sends it identically re-sends it
/// byte-for-byte.
struct CuttingReasoningProvider {
    deltas: usize,
    visible: bool,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}
#[async_trait]
impl ChatProvider for CuttingReasoningProvider {
    fn name(&self) -> &str {
        "cutting-reasoning"
    }
    fn retry_delays(&self) -> Vec<std::time::Duration> {
        vec![std::time::Duration::from_millis(1); 8]
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
        let delta = "x".repeat(CUT_DELTA_BYTES);
        let mut events = Vec::with_capacity(self.deltas + 1);
        for _ in 0..self.deltas {
            events.push(Ok(if self.visible {
                ProviderEvent::TextDelta(delta.clone())
            } else {
                ProviderEvent::ReasoningDelta(delta.clone())
            }));
        }
        events.push(Err(saya_agent::ProviderError::StreamEnded));
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}
