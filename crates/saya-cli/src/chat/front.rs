//! The surface a chat session drives, and the only thing it talks to.
//!
//! One front per session, shared by the root turn and every helper: it owns the
//! engine event sink the turn is built with, answers the calls put to the
//! operator, and receives the session's own events. What any of it looks like
//! is the front's business — the session never renders and never prompts.

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::AgentEventSink;

use super::{ChatEvent, Question};
use crate::agent::tools::consent::Answer;

/// The surface one session drives.
#[async_trait]
pub(crate) trait ChatFront: Send + Sync {
    /// The sink a turn is built with: the engine's own stream, so text, tool
    /// steps and metrics reach the surface without the session re-emitting
    /// them.
    fn sink(&self) -> Arc<dyn AgentEventSink>;

    /// Puts one call to the user and returns the answer. A front with nobody
    /// to ask — no terminal, a non-interactive run — answers [`Answer::Deny`]
    /// without waiting.
    async fn ask(&self, question: &Question) -> Answer;

    /// Delivers one session event to the surface.
    fn emit(&self, event: ChatEvent);
}
