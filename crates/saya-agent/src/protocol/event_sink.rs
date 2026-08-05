use crate::AgentEvent;
use async_trait::async_trait;
#[async_trait]
pub trait AgentEventSink: Send + Sync {
    async fn emit(&self, event: AgentEvent);
}
pub struct NoopEventSink;
#[async_trait]
impl AgentEventSink for NoopEventSink {
    async fn emit(&self, _: AgentEvent) {}
}
