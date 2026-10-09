struct CancellingSink {
    token: CancellationToken,
    events: Arc<Mutex<Vec<AgentEvent>>>,
}
#[async_trait]
impl AgentEventSink for CancellingSink {
    async fn emit(&self, event: AgentEvent) {
        self.events.lock().unwrap().push(event.clone());
        if matches!(event, AgentEvent::ToolRequested { .. }) {
            self.token.cancel();
        }
    }
}
