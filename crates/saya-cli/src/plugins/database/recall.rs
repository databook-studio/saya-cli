//! The database plugin's optional recall source.
//!
//! A run that configured memory hands the plugin a [`RecallSource`]: the turn
//! inputs recall reads, everything except the prompt — the prompt is the one
//! thing only the `context()` hook has. The hook recalls through the same
//! [`recall_for_turn`] the chat's turn calls, so the plugin can never recall
//! under a second policy that drifts from the chat's.

use std::sync::Arc;

use saya_agent::ContextBlock;
use saya_config::ResolvedMemory;
use saya_store::SqliteStateStore;

use crate::agent::recall_turn::{RecallTurnInputs, recall_for_turn};
use crate::connection::ConnectionRegistry;
use crate::contracts::RecallReceipt;

/// What a turn's recall needs beyond the prompt, owned so the plugin can hold
/// it across turns. The registry is shared: the turn's own handle is the same
/// connection set, and recall only reads profile identities from it.
pub(crate) struct RecallSource {
    system_prompt: Option<String>,
    allow_query_data: bool,
    memory: ResolvedMemory,
    registry: Arc<ConnectionRegistry>,
    state_db: Option<SqliteStateStore>,
    byte_budget: usize,
}

impl RecallSource {
    pub(crate) fn new(
        system_prompt: Option<String>,
        allow_query_data: bool,
        memory: ResolvedMemory,
        registry: Arc<ConnectionRegistry>,
        state_db: Option<SqliteStateStore>,
        byte_budget: usize,
    ) -> Self {
        Self {
            system_prompt,
            allow_query_data,
            memory,
            registry,
            state_db,
            byte_budget,
        }
    }

    /// Recalls `prompt` for this source's turn: the owned inputs become the
    /// borrows the shared function takes.
    pub(crate) async fn recall(&self, prompt: &str) -> (Vec<ContextBlock>, RecallReceipt) {
        recall_for_turn(RecallTurnInputs {
            prompt,
            system_prompt: self.system_prompt.as_deref(),
            allow_query_data: self.allow_query_data,
            memory: &self.memory,
            registry: &self.registry,
            state_db: self.state_db.as_ref(),
            byte_budget: self.byte_budget,
        })
        .await
    }
}
