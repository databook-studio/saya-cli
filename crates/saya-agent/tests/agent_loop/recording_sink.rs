struct RecordingSink {
    events: Arc<Mutex<Vec<AgentEvent>>>,
}
#[async_trait]
impl AgentEventSink for RecordingSink {
    async fn emit(&self, event: AgentEvent) {
        self.events.lock().unwrap().push(event);
    }
}
