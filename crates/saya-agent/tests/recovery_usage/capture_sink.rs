struct CaptureSink {
    events: Arc<Mutex<Vec<AgentEvent>>>,
    metrics: Arc<Mutex<Vec<AgentMetric>>>,
}
#[async_trait]
impl AgentEventSink for CaptureSink {
    async fn emit(&self, event: AgentEvent) {
        self.events.lock().unwrap().push(event);
    }

    async fn metric(&self, metric: AgentMetric) {
        self.metrics.lock().unwrap().push(metric);
    }
}
