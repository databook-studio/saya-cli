//! The database plugin's `on_turn_finish` hook: it learns through the one
//! function the chat's turn calls, reports that call's usage to the run, and
//! learns nothing at all when no source is attached or the turn was cancelled.
//!
//! These assert the hook's *contract*. The chat path's own behaviour is pinned
//! by the `db_goldens` transcripts, which this change must leave byte-identical.

use std::sync::Arc;

use saya_agent::{RunPlugin, TokenUsage, TurnOutcome};
use saya_store::KnowledgeItemStore;

use super::learning_fixture::{
    MODEL, PROMPT, RecordingSink, ScriptedExtraction, finished, identity, observations, output,
    scratch, source, tools,
};
use super::recall_fixture::{CHAT, chat_plugin};
use crate::agent::learning::after_turn::{LearnAfterTurnInputs, learn_after_turn};
use crate::agent::tools::OverrideLog;
use crate::contracts::RecallReceipt;

/// Test 1 — no learning source attached: the hook contributes no usage and
/// emits nothing, even though the plugin is active.
#[tokio::test]
async fn a_plugin_without_a_learning_source_learns_nothing() {
    let plugin = chat_plugin();
    let sink = RecordingSink::default();
    let output = output();

    let usage = plugin
        .on_turn_finish(&CHAT, &finished(Some(&output)), &sink)
        .await;

    assert_eq!(usage, None, "no source, no usage");
    assert!(
        sink.events().is_empty(),
        "no source, no event: {:?}",
        sink.events()
    );
}

/// Test 2 — with a source, the hook learns exactly what the shared function
/// learns for the same inputs: the same events, the same persisted claim, and
/// that call's usage handed back to the run.
#[tokio::test]
async fn the_hook_learns_what_the_shared_function_learns() {
    let root = scratch("same");
    let identity = identity();
    let output = output();
    let receipt = RecallReceipt::configured_off();

    // The chat's turn, called directly.
    let direct_observations = observations();
    let direct_tools = tools(
        &root.join("direct.sqlite3"),
        &identity,
        direct_observations.clone(),
    );
    let direct_provider = Arc::new(ScriptedExtraction::new());
    let direct_sink = RecordingSink::default();
    let direct_usage = learn_after_turn(
        LearnAfterTurnInputs {
            override_log: Arc::new(OverrideLog::new()),
            cancelled: false,
            permit_candidate_writes: true,
            database: &direct_tools,
            output: Some(&output),
            prompt: PROMPT,
            observations_log: Some(&direct_observations),
            receipt: &receipt,
            provider: direct_provider.as_ref(),
            model: MODEL,
            breaker: None,
        },
        &direct_sink,
    )
    .await;

    // The same turn, through the plugin's hook.
    let hook_observations = observations();
    let hook_tools = tools(
        &root.join("hook.sqlite3"),
        &identity,
        hook_observations.clone(),
    );
    let hook_provider = Arc::new(ScriptedExtraction::new());
    let plugin = chat_plugin().with_learning(source(&hook_tools, hook_observations, hook_provider));
    let hook_sink = RecordingSink::default();
    let hook_usage = plugin
        .on_turn_finish(&CHAT, &finished(Some(&output)), &hook_sink)
        .await;

    assert_eq!(
        hook_usage, direct_usage,
        "the hook reports the extraction call's usage"
    );
    assert_eq!(
        hook_usage,
        Some(TokenUsage::new(4, 2)),
        "the usage is the provider's report for the extraction call"
    );
    assert_eq!(
        hook_sink.events(),
        direct_sink.events(),
        "the hook emits the events the shared function emits, nothing more"
    );

    let direct_claims = claims(&direct_tools, &identity).await;
    assert!(
        !direct_claims.is_empty(),
        "the reply proposed a claim, so the comparison is not two empties: {direct_claims:?}"
    );
    assert_eq!(claims(&hook_tools, &identity).await, direct_claims);

    let _ = std::fs::remove_dir_all(root);
}

/// Test 3 — a cancelled turn learns nothing: the hook makes no extraction
/// request, so the provider sees no call at all.
#[tokio::test]
async fn a_cancelled_turn_makes_no_extraction_request() {
    let root = scratch("cancelled");
    let identity = identity();
    let output = output();
    let observations = observations();
    let tools = tools(&root.join("state.sqlite3"), &identity, observations.clone());
    let provider = Arc::new(ScriptedExtraction::new());
    let plugin = chat_plugin().with_learning(source(&tools, observations, provider.clone()));
    let sink = RecordingSink::default();

    let usage = plugin
        .on_turn_finish(
            &CHAT,
            &TurnOutcome {
                answered: false,
                cancelled: true,
                prompt: PROMPT,
                output: Some(&output),
            },
            &sink,
        )
        .await;

    assert_eq!(usage, None, "a cancelled turn reports no learning usage");
    assert_eq!(
        provider.calls(),
        0,
        "no extraction request reaches the provider"
    );
    assert!(
        sink.events().is_empty(),
        "nothing is announced for a turn that does not learn: {:?}",
        sink.events()
    );

    let _ = std::fs::remove_dir_all(root);
}

/// The claims a run persisted, as the slots a store holds for the profile.
async fn claims(
    tools: &Arc<crate::agent::tools::DatabaseTools>,
    identity: &saya_types::ProfileIdentity,
) -> Vec<String> {
    let store = tools.state_db().expect("the harness attaches a store");
    store
        .knowledge_for_profile(identity)
        .await
        .expect("items listed")
        .into_iter()
        .map(|item| format!("{:?}", item.slot))
        .collect()
}
