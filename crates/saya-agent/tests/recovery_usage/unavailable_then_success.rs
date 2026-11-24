/// A provider whose first send never answers — the shape a lost connection or a
/// gateway 5xx leaves behind — and whose retry answers. The interactive and
/// `ask` surfaces drive this same receive path, so the pin is theirs too.
struct UnavailableThenSuccess {
    attempts: Mutex<usize>,
}
#[async_trait]
impl ChatProvider for UnavailableThenSuccess {
    fn name(&self) -> &str {
        "unavailable-then-success"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the agent receive path streams")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        let mut attempts = self.attempts.lock().unwrap();
        *attempts += 1;
        if *attempts == 1 {
            return Err(ProviderError::Unavailable(
                "the connection was lost before a response".into(),
            ));
        }
        Ok(Box::pin(futures_util::stream::iter(vec![
            Ok(ProviderEvent::TextDelta("recovered".into())),
            Ok(ProviderEvent::Done),
        ])))
    }
}
