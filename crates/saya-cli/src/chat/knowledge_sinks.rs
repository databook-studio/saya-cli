//! The two collaborators the knowledge-event twins run with: the post-turn
//! extractor the learning pass calls, and the sink that records every event the
//! turn emitted.
//!
//! [`Recording`] also answers the ordering question the old body's provider-order
//! row asked. It holds the scripted provider and reads its arrival count *inside*
//! `emit`: a count of zero at the instant the first `KnowledgeSupplied` arrived
//! is the proof the emit preceded the request.

use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, ChatMessage, ChatProvider, ChatRequest, ChatResponse,
    KnowledgeOutcome, OverrideFindingDto, ProviderError,
};

use super::knowledge_fixture::NO_PROPOSALS;
use crate::commands::coding::parent_turn_provider::Scripted;

/// The provider the post-turn learning pass calls: it counts its calls and
/// answers every one with `reply`.
pub(super) struct Extractor {
    reply: Option<String>,
    calls: Mutex<usize>,
}

impl Extractor {
    pub(super) fn new(reply: Option<String>) -> Self {
        Self {
            reply,
            calls: Mutex::new(0),
        }
    }

    /// Whether this extractor is attached to a turn at all.
    pub(super) fn attached(&self) -> bool {
        self.reply.is_some()
    }

    /// How many extraction calls arrived: zero means the turn never asked.
    pub(super) fn calls(&self) -> usize {
        *self.calls.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[async_trait]
impl ChatProvider for Extractor {
    fn name(&self) -> &str {
        "scripted-extractor"
    }

    async fn complete(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        *self.calls.lock().unwrap_or_else(PoisonError::into_inner) += 1;
        let reply = self
            .reply
            .clone()
            .unwrap_or_else(|| NO_PROPOSALS.to_owned());
        Ok(ChatResponse::new(ChatMessage::text("assistant", reply)))
    }
}

/// The front's own sink: every event the turn emitted, in order, and the
/// provider's arrival count seen when the first `KnowledgeSupplied` arrived.
#[derive(Clone, Default)]
pub(super) struct Recording {
    events: Arc<Mutex<Vec<AgentEvent>>>,
    /// The scripted provider, when the test watches the emit's order against
    /// the request: the count is read inside `emit`, before one exists.
    provider: Option<Arc<Scripted>>,
    arrived_at_supplied: Arc<Mutex<Option<usize>>>,
}

impl Recording {
    /// A sink that also records how many provider requests had arrived when the
    /// turn's `KnowledgeSupplied` was emitted.
    pub(super) fn watching(provider: Arc<Scripted>) -> Self {
        Self {
            provider: Some(provider),
            ..Self::default()
        }
    }

    pub(super) fn sink(&self) -> Arc<dyn AgentEventSink> {
        Arc::new(self.clone())
    }

    pub(super) fn events(&self) -> Vec<AgentEvent> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Every `KnowledgeSupplied`, in order.
    pub(super) fn supplied(&self) -> Vec<AgentEvent> {
        self.events()
            .into_iter()
            .filter(|event| matches!(event, AgentEvent::KnowledgeSupplied { .. }))
            .collect()
    }

    /// What the turn's first `KnowledgeSupplied` said, when one arrived.
    pub(super) fn supplied_outcome(&self) -> Option<KnowledgeOutcome> {
        self.events().into_iter().find_map(|event| match event {
            AgentEvent::KnowledgeSupplied { outcome, .. } => Some(outcome),
            _ => None,
        })
    }

    /// The override findings the turn announced, when it announced any.
    pub(super) fn overridden(&self) -> Option<Vec<OverrideFindingDto>> {
        self.events().into_iter().find_map(|event| match event {
            AgentEvent::KnowledgeOverridden { findings } => Some(findings),
            _ => None,
        })
    }

    /// How many provider requests had arrived when the first
    /// `KnowledgeSupplied` was emitted.
    pub(super) fn arrived_at_supplied(&self) -> Option<usize> {
        *self
            .arrived_at_supplied
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

#[async_trait]
impl AgentEventSink for Recording {
    async fn emit(&self, event: AgentEvent) {
        if matches!(event, AgentEvent::KnowledgeSupplied { .. })
            && let Some(provider) = self.provider.as_ref()
        {
            let mut slot = self
                .arrived_at_supplied
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if slot.is_none() {
                *slot = Some(provider.arrived());
            }
        }
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }
}
