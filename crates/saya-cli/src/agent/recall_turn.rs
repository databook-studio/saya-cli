//! One turn's recall, as a function.
//!
//! The chat path and the database plugin's `context()` hook both recall the
//! same way for the same turn: off stays off, a closed privacy gate returns
//! before the store is touched, and only then does recall run. Keeping that
//! decision in one place means a surface cannot drift into a second policy —
//! the plugin's hook is the same function with the same inputs.
//!
//! [`RecallTurnInputs`] is everything a turn's recall reads. The chat's turn
//! borrows each field from its own state; the plugin's source owns them and
//! hands out borrows of the same shape.

use saya_agent::ContextBlock;
use saya_config::ResolvedMemory;
use saya_store::SqliteStateStore;

use crate::connection::ConnectionRegistry;
use crate::contracts::RecallReceipt;

/// What a turn's recall reads. Every field is a turn input, not a policy of
/// its own: the mode and the bounds come from `memory`, the gate from
/// `allow_query_data`, and the connection set from `registry`.
pub(crate) struct RecallTurnInputs<'a> {
    /// The user's question — what recall selects against and what the byte
    /// bound must leave room for.
    pub(crate) prompt: &'a str,
    /// The system prompt already assembled for this turn, when there is one.
    pub(crate) system_prompt: Option<&'a str>,
    /// Whether this turn may read stored database knowledge at all.
    pub(crate) allow_query_data: bool,
    /// The resolved `[memory]` settings: the mode and the recall bounds.
    pub(crate) memory: &'a ResolvedMemory,
    /// The profiles that actually connected — where identities come from.
    pub(crate) registry: &'a ConnectionRegistry,
    /// The state store, or `None` when this run has none.
    pub(crate) state_db: Option<&'a SqliteStateStore>,
    /// The conversation's context byte budget, which bounds the rendered block.
    pub(crate) byte_budget: usize,
}

/// Recalls this turn's context blocks and the receipt naming what reached them.
///
/// Off is off — no block and a `ConfiguredOff` receipt, without asking the
/// store. A closed privacy gate returns before any store query, with a
/// `PrivacyGateClosed` receipt, which is a different fact from running and
/// finding nothing. Otherwise [`super::recall_context::recall_context_blocks`]
/// decides what fits.
pub(crate) async fn recall_for_turn(
    inputs: RecallTurnInputs<'_>,
) -> (Vec<ContextBlock>, RecallReceipt) {
    let RecallTurnInputs {
        prompt,
        system_prompt,
        allow_query_data,
        memory,
        registry,
        state_db,
        byte_budget,
    } = inputs;
    match super::learning::recall_mode_for(memory.mode) {
        None => (Vec::new(), RecallReceipt::configured_off()),
        Some(_) if !allow_query_data => (Vec::new(), RecallReceipt::privacy_gate_closed()),
        Some(mode) => {
            super::recall_context::recall_context_blocks(
                prompt,
                system_prompt,
                allow_query_data,
                mode,
                super::learning::bounds_from(memory),
                registry,
                state_db,
                byte_budget,
            )
            .await
        }
    }
}
