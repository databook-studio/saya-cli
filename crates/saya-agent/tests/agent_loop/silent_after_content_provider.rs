/// A provider whose first stream delivers one content delta and then goes
/// silent, and whose retry answers. The wire shape behind this is a gateway
/// that opens the stream, sends one delta, and then only writes SSE comment
/// keep-alives (`: OPENROUTER PROCESSING`); the provider parser drops those
/// before the loop, so the loop sees a stream that stopped producing content
/// while its transport stays alive. Nothing but a content-progress clock can
/// end that.
struct SilentAfterContentProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}
#[async_trait]
impl ChatProvider for SilentAfterContentProvider {
    fn name(&self) -> &str {
        "silent-after-content"
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
        if attempt > 0 {
            return Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(ProviderEvent::TextDelta("The answer is 42.".into())),
                Ok(ProviderEvent::Done),
            ])));
        }
        // One delta, then nothing at all: the retried events never arrive, and
        // the stream never ends either.
        let head = futures_util::stream::iter(vec![Ok(ProviderEvent::TextDelta(
            "The an".into(),
        ))]);
        Ok(Box::pin(head.chain(futures_util::stream::pending())))
    }
}
