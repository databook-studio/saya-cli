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
}
