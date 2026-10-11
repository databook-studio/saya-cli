/// Content-free fact from an actual context compaction. Bytes use the loop's
/// context-size estimator, not exact wire bytes or provider token counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct ContextCompaction {
    pub before_bytes: u64,
    pub after_bytes: u64,
    pub evicted_tool_groups: u64,
    pub truncated_tool_messages: u64,
    pub protected_prefix_messages: u64,
    pub protected_prefix_unchanged: bool,
}

use crate::{AgentEvent, AgentMetric, AnsweringObservation};
use async_trait::async_trait;
#[async_trait]
pub trait AgentEventSink: Send + Sync {
    /// Called synchronously after mutation, never for a dry-run or a no-op.
    /// Observers own bounded persistence/delivery and must surface missing facts.
    fn observe_compaction(&self, _observation: ContextCompaction) {}

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
