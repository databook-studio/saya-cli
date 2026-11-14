struct RecordingSink(Arc<Mutex<Vec<AgentEvent>>>);
#[async_trait]
impl AgentEventSink for RecordingSink {
    async fn emit(&self, event: AgentEvent) {
        self.0.lock().unwrap().push(event);
    }
}
