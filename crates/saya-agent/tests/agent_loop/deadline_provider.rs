/// A provider whose first answer flows and then never ends. The loop's own
/// per-request deadline must cut it; the second call answers normally, so the
/// run can only finish if the cut is recovered as a bounded continuation.
struct DeadlineProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl ChatProvider for DeadlineProvider {
    fn name(&self) -> &str {
        "deadline"
    }
    fn request_timeout(&self) -> Duration {
        Duration::from_millis(80)
    }
    async fn complete(
        &self,
        _: ChatRequest,
    ) -> Result<ChatResponse, saya_agent::ProviderError> {
        unreachable!("the main loop drives the provider through stream")
    }
    async fn stream(
        &self,
        request: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, saya_agent::ProviderError> {
        self.requests.lock().unwrap().push(request);
        if self
            .calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            == 0
        {
            let first = futures_util::stream::once(async {
                Ok(ProviderEvent::TextDelta("partial answer".into()))
            });
            return Ok(Box::pin(first.chain(futures_util::stream::pending())));
        }
        Ok(Box::pin(futures_util::stream::iter(vec![
            Ok(ProviderEvent::TextDelta("final answer".into())),
            Ok(ProviderEvent::Done),
        ])))
    }
}
