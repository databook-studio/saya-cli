struct CancelOnRetry {
    cancellation: CancellationToken,
    metrics: Arc<Mutex<Vec<AgentMetric>>>,
}
#[async_trait]
impl AgentEventSink for CancelOnRetry {
    async fn emit(&self, event: AgentEvent) {
        if matches!(
            event,
            AgentEvent::ProviderRecovery {
                phase: ProviderRecoveryPhase::Retrying,
                ..
            }
        ) {
            self.cancellation.cancel();
        }
    }

    async fn metric(&self, metric: AgentMetric) {
        self.metrics.lock().unwrap().push(metric);
    }
}
