//! The three-state recall outcome on the live chat turn, plus the store that
//! could not be read.
//!
//! The old body's rows (`agent/runtime_tests/outcome_states.rs`,
//! `runtime_posture.rs`, `runtime_posture/privacy_gate.rs`,
//! `event_persistence.rs`) all assert the same contract: the receipt is the
//! single source of truth for what the turn says about memory, and a store that
//! fails to answer is *named*, never silently read as "nothing was found".

#![cfg(unix)]

use saya_agent::{AgentEvent, KnowledgeOutcome};
use saya_config::MemoryMode;
use saya_types::ClaimStatus;

use super::knowledge_fixture::{self, QUESTION, Wanted};
use super::knowledge_sinks::Recording;
use crate::commands::coding::parent_turn_provider::{Reply, Scripted};

/// Recall configured off: SAYA did not look. Distinct from Skipped and Ran.
#[tokio::test]
async fn a_live_turn_with_recall_off_emits_the_off_outcome() {
    let provider = Scripted::start_with_script(vec![Reply::Text("done")]);
    let case = knowledge_fixture::build(
        "knowledge-off",
        &provider.endpoint,
        Wanted {
            memory: MemoryMode::Off,
            claim: None,
            extractor: None,
            ..Wanted::default()
        },
    )
    .await;
    let sink = Recording::default();
    let turn = case.turn(&sink, QUESTION).await;
    assert!(turn.is_ok(), "the turn must run: {turn:?}");

    let supplied = sink.supplied();
    assert_eq!(supplied.len(), 1, "one event per turn: {supplied:?}");
    let AgentEvent::KnowledgeSupplied {
        outcome, contracts, ..
    } = &supplied[0]
    else {
        unreachable!("the filtered event is KnowledgeSupplied")
    };
    assert_eq!(outcome, &KnowledgeOutcome::Off);
    assert!(contracts.is_empty(), "off supplies nothing: {contracts:?}");
}

/// A closed privacy gate: SAYA was not permitted to look, which is a different
/// fact from recall being off. The store is never touched.
#[tokio::test]
async fn a_live_turn_with_the_privacy_gate_closed_emits_the_skipped_outcome() {
    let provider = Scripted::start_with_script(vec![Reply::Text("done")]);
    let case = knowledge_fixture::build(
        "knowledge-gate-closed",
        &provider.endpoint,
        Wanted {
            allow_query_data: false,
            ..Wanted::default()
        },
    )
    .await;
    let sink = Recording::default();
    let turn = case.turn(&sink, QUESTION).await;
    assert!(turn.is_ok(), "the turn must run: {turn:?}");

    let event = sink.supplied_outcome().expect("KnowledgeSupplied present");
    assert_eq!(event, KnowledgeOutcome::Skipped);
    let AgentEvent::KnowledgeSupplied { contracts, .. } = &sink.supplied()[0] else {
        unreachable!("the filtered event is KnowledgeSupplied")
    };
    assert!(
        contracts.is_empty(),
        "a closed gate supplies nothing: {contracts:?}"
    );
}

/// Recall ran and matched nothing: Ran with an empty supplied list — the third
/// state, distinct from Off (did not look) and Skipped (not allowed to look).
#[tokio::test]
async fn a_live_turn_whose_recall_ran_and_found_nothing_emits_ran() {
    let provider = Scripted::start_with_script(vec![Reply::Text("done")]);
    let case = knowledge_fixture::build(
        "knowledge-ran-empty",
        &provider.endpoint,
        Wanted {
            claim: None,
            ..Wanted::default()
        },
    )
    .await;
    let sink = Recording::default();
    let turn = case.turn(&sink, QUESTION).await;
    assert!(turn.is_ok(), "the turn must run: {turn:?}");

    let supplied = sink.supplied();
    assert_eq!(supplied.len(), 1, "one event per turn: {supplied:?}");
    let AgentEvent::KnowledgeSupplied {
        outcome, contracts, ..
    } = &supplied[0]
    else {
        unreachable!("the filtered event is KnowledgeSupplied")
    };
    assert_eq!(
        outcome,
        &KnowledgeOutcome::Ran {
            store_unavailable: false
        }
    );
    assert!(contracts.is_empty(), "nothing was found: {contracts:?}");
}

/// A store that cannot be read still runs the turn and still says so: Ran with
/// `store_unavailable: true`, which is not silence and not "found nothing".
#[tokio::test]
async fn a_live_turn_whose_store_is_unavailable_still_emits() {
    let provider = Scripted::start_with_script(vec![Reply::Text("done anyway")]);
    let case = knowledge_fixture::build(
        "knowledge-store-unavailable",
        &provider.endpoint,
        Wanted {
            claim: Some(ClaimStatus::Confirmed),
            ..Wanted::default()
        },
    )
    .await;
    // Recall's own read is the one made to fail: the knowledge table the store
    // keeps it in is dropped, while every table the run writes stays intact.
    case.make_store_unavailable().await;
    let sink = Recording::default();
    let result = case.turn(&sink, QUESTION).await;
    assert!(
        result.is_ok(),
        "turn completes despite store failure: {result:?}"
    );

    let outcome = sink
        .supplied_outcome()
        .expect("KnowledgeSupplied emitted despite store failure");
    assert_eq!(
        outcome,
        KnowledgeOutcome::Ran {
            store_unavailable: true
        },
        "store failure is Ran{{store_unavailable: true}}, not silence: {:?}",
        sink.events()
    );
}
