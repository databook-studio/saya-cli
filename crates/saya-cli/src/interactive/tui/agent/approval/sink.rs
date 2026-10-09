//! The sink that relays agent events onto the UI channel.

use super::super::messages::StreamMsg;
use async_trait::async_trait;
use saya_agent::{AgentEvent, AgentEventSink};
use tokio::sync::mpsc::UnboundedSender;

/// Sink that forwards every agent event to the UI channel.
pub(crate) struct ChannelSink {
    pub(crate) tx: UnboundedSender<StreamMsg>,
}

#[async_trait]
impl AgentEventSink for ChannelSink {
    async fn emit(&self, event: AgentEvent) {
        let _ = self.tx.send(StreamMsg::Event(event));
    }
}
