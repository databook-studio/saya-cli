//! The old turn body's `KnowledgeSupplied` pins, twinned on the live chat turn.
//!
//! The regression PT2 found: the chat recalls through the database plugin's
//! `context()` hook and kept the receipt for the learning pass, but never turned
//! it into the one event the surfaces show — so a chat session said nothing
//! about what memory supplied. These rows are the old body's own sentences
//! (`agent/runtime_tests/supplied_event.rs`, `provider_order.rs`,
//! `outcome_states.rs`, `event_persistence.rs`) on a composed session's turn.

#![cfg(unix)]

use std::sync::Arc;

use saya_agent::AgentEvent;
use saya_types::ClaimStatus;

use super::knowledge_fixture::{self, CLAIMED_COLUMN, QUESTION, Wanted};
use super::knowledge_sinks::Recording;
use crate::commands::coding::parent_turn_provider::{Reply, Scripted};

/// One claim supplied this turn emits exactly one event naming it — the claim,
/// its status, and the profile's human-facing name, never its identity.
#[tokio::test]
async fn a_live_chat_turn_supplying_claims_emits_one_event_naming_those_claims() {
    let provider = Scripted::start_with_script(vec![Reply::Text("done")]);
    let case = knowledge_fixture::build(
        "knowledge-supplied-one",
        &provider.endpoint,
        Wanted::default(),
    )
    .await;
    let sink = Recording::default();
    let turn = case.turn(&sink, QUESTION).await;
    assert!(turn.is_ok(), "the turn must run: {turn:?}");

    let supplied = sink.supplied();
    assert_eq!(
        supplied.len(),
        1,
        "exactly one KnowledgeSupplied: {supplied:?}"
    );
    let AgentEvent::KnowledgeSupplied {
        outcome,
        contracts,
        dropped_by_bounds,
    } = &supplied[0]
    else {
        unreachable!("the filtered event is KnowledgeSupplied")
    };
    assert_eq!(
        outcome,
        &saya_agent::KnowledgeOutcome::Ran {
            store_unavailable: false
        },
        "recall ran against the session's store"
    );
    assert_eq!(contracts.len(), 1, "one object supplied: {contracts:?}");
    assert_eq!(contracts[0].object, case.object());
    assert!(
        contracts[0].object.ends_with(".orders"),
        "the supplied object is the claimed table: {}",
        contracts[0].object
    );
    assert_eq!(contracts[0].profile, "primary", "name, not identity");
    assert_eq!(contracts[0].claims.len(), 1);
    assert_eq!(contracts[0].claims[0].claim_id, case.claim_id());
    assert_eq!(contracts[0].claims[0].value, CLAIMED_COLUMN);
    assert_eq!(contracts[0].claims[0].status, ClaimStatus::Confirmed);
    assert_eq!(*dropped_by_bounds, 0, "nothing was dropped by the bounds");

    // The event serializes under the tag a surface reads it by.
    let json = serde_json::to_string(&supplied[0]).expect("the event serializes");
    assert!(
        json.contains(r#""type":"knowledge_supplied""#),
        "carries the type tag: {json}"
    );
}

/// The emit precedes the provider request: at the instant the front's sink
/// received the event, no request had arrived at the provider — and in the
/// event sequence it comes before the first thing the loop said.
#[tokio::test]
async fn knowledge_supplied_precedes_the_provider_request() {
    let provider = Arc::new(Scripted::start_with_script(vec![Reply::Text("done")]));
    let case = knowledge_fixture::build(
        "knowledge-supplied-order",
        &provider.endpoint,
        Wanted::default(),
    )
    .await;
    let sink = Recording::watching(Arc::clone(&provider));
    let turn = case.turn(&sink, QUESTION).await;
    assert!(turn.is_ok(), "the turn must run: {turn:?}");

    assert_eq!(
        sink.arrived_at_supplied(),
        Some(0),
        "KnowledgeSupplied was emitted before the provider was called"
    );
    let captured = sink.events();
    let knowledge_idx = captured
        .iter()
        .position(|event| matches!(event, AgentEvent::KnowledgeSupplied { .. }))
        .expect("KnowledgeSupplied in the turn's stream");
    let first_loop_idx = captured
        .iter()
        .position(|event| {
            matches!(
                event,
                AgentEvent::AssistantText { .. }
                    | AgentEvent::ToolRequested { .. }
                    | AgentEvent::Complete
            )
        })
        .expect("a loop event after the provider call");
    assert!(
        knowledge_idx < first_loop_idx,
        "KnowledgeSupplied (idx {knowledge_idx}) must precede the first loop event (idx {first_loop_idx}): {captured:?}"
    );
}

/// A candidate claim is supplied unflattened: its status rides the event as
/// Candidate, distinct from a confirmed claim's.
#[tokio::test]
async fn a_live_candidate_claims_status_survives_into_the_event() {
    let provider = Scripted::start_with_script(vec![Reply::Text("done")]);
    let case = knowledge_fixture::build(
        "knowledge-supplied-candidate",
        &provider.endpoint,
        Wanted {
            claim: Some(ClaimStatus::Candidate),
            ..Wanted::default()
        },
    )
    .await;
    let sink = Recording::default();
    let turn = case.turn(&sink, QUESTION).await;
    assert!(turn.is_ok(), "the turn must run: {turn:?}");

    let supplied = sink.supplied();
    let AgentEvent::KnowledgeSupplied { contracts, .. } =
        supplied.first().expect("KnowledgeSupplied present")
    else {
        unreachable!("the filtered event is KnowledgeSupplied")
    };
    assert_eq!(contracts.len(), 1);
    assert_eq!(contracts[0].claims.len(), 1);
    assert_eq!(contracts[0].claims[0].claim_id, case.claim_id());
    // The candidate status survives as Candidate, not flattened to confirmed.
    assert_eq!(contracts[0].claims[0].status, ClaimStatus::Candidate);
    assert_ne!(contracts[0].claims[0].status, ClaimStatus::Confirmed);
}

/// No opaque profile identity value appears in the turn's serialized events:
/// the receipt carries the human-facing name only, and the event has no
/// identity field by construction.
#[tokio::test]
async fn no_opaque_profile_identity_value_appears_in_the_live_event() {
    let provider = Scripted::start_with_script(vec![Reply::Text("done")]);
    let case = knowledge_fixture::build(
        "knowledge-supplied-identity",
        &provider.endpoint,
        Wanted::default(),
    )
    .await;
    let identity = case.identity();
    let sink = Recording::default();
    let turn = case.turn(&sink, QUESTION).await;
    assert!(turn.is_ok(), "the turn must run: {turn:?}");

    let json = serde_json::to_string(&sink.events()).expect("the events serialize");
    assert!(
        !json.contains(identity.as_str()),
        "opaque identity leaked into the event stream: {json}"
    );
    assert!(
        json.contains("\"primary\""),
        "the profile name (not the identity) is what the event carries: {json}"
    );
}
