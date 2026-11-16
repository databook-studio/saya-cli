/// A sink that records nothing, for tests where the events are not under test.
struct NoopSink;
#[async_trait]
impl AgentEventSink for NoopSink {
    async fn emit(&self, _: AgentEvent) {}
}
