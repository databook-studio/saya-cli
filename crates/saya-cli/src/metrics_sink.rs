use crate::local_metrics::LocalMetrics;
use async_trait::async_trait;
use saya_agent::{AgentEvent, AgentEventSink, AgentMetric, AnsweringObservation};
use std::sync::Arc;

pub(crate) struct MetricsSink<'a> {
    inner: &'a dyn AgentEventSink,
    metrics: Option<Arc<LocalMetrics>>,
}

impl<'a> MetricsSink<'a> {
    pub(crate) fn new(inner: &'a dyn AgentEventSink, metrics: Option<Arc<LocalMetrics>>) -> Self {
        Self { inner, metrics }
    }
}

#[async_trait]
impl AgentEventSink for MetricsSink<'_> {
    async fn emit(&self, event: AgentEvent) {
        self.inner.emit(event).await;
    }

    async fn observe_answering(&self, observation: AnsweringObservation) {
        self.inner.observe_answering(observation).await;
    }

    async fn metric(&self, metric: AgentMetric) {
        if let Some(metrics) = &self.metrics {
            metrics.record(&metric);
        }
        self.inner.metric(metric).await;
    }

    async fn background_exit(&self, text: &str) {
        self.inner.background_exit(text).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    struct ExitSink(Mutex<Vec<String>>);

    #[async_trait]
    impl AgentEventSink for ExitSink {
        async fn emit(&self, _event: AgentEvent) {}

        async fn background_exit(&self, text: &str) {
            self.0.lock().expect("exit log lock").push(text.to_owned());
        }
    }

    /// The TUI's display sink sits behind this wrapper: the exit notice has
    /// to reach it, not stop here.
    #[tokio::test]
    async fn the_exit_notice_crosses_the_wrapper() {
        let inner = ExitSink(Mutex::new(Vec::new()));
        let wrapped = MetricsSink::new(&inner, None);
        wrapped.background_exit("background shell 2 exited").await;
        assert_eq!(
            inner.0.lock().expect("exit log lock").as_slice(),
            ["background shell 2 exited"]
        );
    }
}
