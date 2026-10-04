use crate::{AgentEvent, AgentMetric, AnsweringObservation};
use async_trait::async_trait;
#[async_trait]
pub trait AgentEventSink: Send + Sync {
    async fn emit(&self, event: AgentEvent);

    /// Receives source-owned admission counts, separate from display events.
    /// A sink that cannot persist an observation should cancel its shared token
    /// with `CancellationReason::OwnerFailure` before returning.
    async fn observe_answering(&self, _observation: AnsweringObservation) {}

    /// Receives fixed source-owned metrics, separate from display events.
    async fn metric(&self, _metric: AgentMetric) {}
}
pub struct NoopEventSink;
#[async_trait]
impl AgentEventSink for NoopEventSink {
    async fn emit(&self, _: AgentEvent) {}
}
