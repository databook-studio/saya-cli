struct MetricSink(Arc<Mutex<Vec<AgentMetric>>>);
#[async_trait]
impl AgentEventSink for MetricSink {
    async fn emit(&self, _: AgentEvent) {}

    async fn metric(&self, metric: AgentMetric) {
        self.0.lock().unwrap().push(metric);
    }
}
