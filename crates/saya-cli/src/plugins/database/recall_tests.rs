//! The database plugin's `context()` hook: it recalls through the one function
//! the chat's turn calls, and keeps the receipt the executor later reads.
//!
//! These assert the hook's *contract* — same blocks, same receipt, and the two
//! degenerate cases (memory off, privacy gate closed) that must never touch the
//! store. The chat path's end-to-end behaviour is pinned by the `db_goldens`
//! transcripts, which this change must leave byte-identical.

use std::sync::Arc;

use saya_agent::RunPlugin;
use saya_config::MemoryMode;
use saya_store::SqliteStateStore;

use super::RecallSource;
use super::recall_fixture::{
    BYTE_BUDGET, CHAT, PROMPT, blocks_view, chat_plugin, identity_for, memory, registry_for,
};
use crate::agent::recall_turn::{RecallTurnInputs, recall_for_turn};
use crate::contracts::RecallOutcomeKind;

/// Test 1 — the hook supplies exactly what the shared function supplies for the
/// same inputs, and keeps that function's receipt.
#[tokio::test]
async fn the_hook_recalls_what_the_shared_function_recalls() {
    let root = super::recall_fixture::scratch("same");
    let identity = identity_for("analytics");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    super::recall_fixture::seed::seed_confirmed_claim(&store, &identity).await;
    let registry = Arc::new(registry_for("analytics", &identity));
    let memory = memory(MemoryMode::Assisted);

    let (expected_blocks, expected_receipt) = recall_for_turn(RecallTurnInputs {
        prompt: PROMPT,
        system_prompt: None,
        allow_query_data: true,
        memory: &memory,
        registry: &registry,
        state_db: Some(&store),
        byte_budget: BYTE_BUDGET,
    })
    .await;
    assert!(
        !expected_blocks.is_empty(),
        "the seeded claim reaches a block, so the comparison is not two empties"
    );

    let plugin = chat_plugin().with_recall(RecallSource::new(
        None,
        true,
        memory,
        registry,
        Some(store),
        BYTE_BUDGET,
    ));
    let blocks = plugin.context(&CHAT, PROMPT).await;

    assert_eq!(
        blocks_view(&blocks),
        blocks_view(&expected_blocks),
        "the hook's blocks are byte-equal to the function's"
    );
    assert_eq!(
        plugin.last_receipt().as_deref(),
        Some(&expected_receipt),
        "the hook keeps the function's receipt for the executor"
    );

    let _ = std::fs::remove_dir_all(root);
}

/// Test 2a — memory off: no block, and the receipt says configuration decided,
/// not that recall looked and found nothing.
#[tokio::test]
async fn memory_off_supplies_no_block_and_reports_configured_off() {
    let identity = identity_for("analytics");
    let plugin = chat_plugin().with_recall(RecallSource::new(
        None,
        true,
        memory(MemoryMode::Off),
        Arc::new(registry_for("analytics", &identity)),
        None,
        BYTE_BUDGET,
    ));

    let blocks = plugin.context(&CHAT, PROMPT).await;

    assert!(blocks.is_empty(), "off supplies nothing: {blocks:?}");
    let receipt = plugin
        .last_receipt()
        .expect("the hook records that recall was configured off");
    assert_eq!(receipt.kind, RecallOutcomeKind::ConfiguredOff);
}

/// Test 2b — the privacy gate closed: no block, and a receipt distinct from
/// both off and ran-and-found-nothing.
#[tokio::test]
async fn a_closed_privacy_gate_supplies_no_block_and_reports_the_gate() {
    let root = super::recall_fixture::scratch("gate");
    let identity = identity_for("analytics");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    super::recall_fixture::seed::seed_confirmed_claim(&store, &identity).await;
    let plugin = chat_plugin().with_recall(RecallSource::new(
        None,
        false,
        memory(MemoryMode::Assisted),
        Arc::new(registry_for("analytics", &identity)),
        Some(store),
        BYTE_BUDGET,
    ));

    let blocks = plugin.context(&CHAT, PROMPT).await;

    assert!(
        blocks.is_empty(),
        "a closed gate supplies nothing: {blocks:?}"
    );
    let receipt = plugin
        .last_receipt()
        .expect("the hook records that the gate was closed");
    assert_eq!(receipt.kind, RecallOutcomeKind::PrivacyGateClosed);

    let _ = std::fs::remove_dir_all(root);
}

/// Test 3 — a plugin with no recall source contributes nothing and claims no
/// receipt, so the executor cannot read one that was never made.
#[tokio::test]
async fn a_plugin_without_a_source_supplies_nothing_and_keeps_no_receipt() {
    let plugin = chat_plugin();

    let blocks = plugin.context(&CHAT, PROMPT).await;

    assert!(blocks.is_empty(), "no source, no block: {blocks:?}");
    assert!(plugin.last_receipt().is_none(), "no source, no receipt");
}
