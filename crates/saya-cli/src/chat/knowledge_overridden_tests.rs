//! `KnowledgeOverridden` on the live chat turn.
//!
//! The emitter has lived in the post-turn learning pass since the chat moved
//! onto the job engine (`agent/learning/after_turn.rs`), but nothing pinned it:
//! PT2's GAP 2. These are the old body's five findings rows
//! (`agent/runtime_tests/overrides_confirmed.rs`, `overrides_candidates.rs`)
//! plus the anti-self-reinforcement row (`contract_tools.rs`), driven over a
//! composed session whose model actually generates the SQL.

#![cfg(unix)]

use saya_types::ClaimStatus;
use serde_json::json;

use super::knowledge_fixture::{
    self, CLAIMED_COLUMN, HONOURING_SQL, OTHER_COLUMN, OVERRIDE_SQL, QUESTION, UNPARSEABLE_SQL,
    Wanted,
};
use super::knowledge_sinks::Recording;
use crate::commands::coding::parent_turn_provider::{Reply, Scripted};

/// The scripted turn: one `bounded_sql_query` call with `sql`, then the text
/// that ends it. A query that names no connection runs on the session's
/// primary.
fn turn_with(sql: &'static str) -> Vec<Reply> {
    vec![
        Reply::Call {
            name: "bounded_sql_query",
            arguments: json!({ "sql": sql }),
        },
        Reply::Text("done"),
    ]
}

/// A statement that contradicts a supplied confirmed claim emits exactly one
/// finding, naming the claim and the time-named column the SQL referenced
/// instead.
#[tokio::test]
async fn a_live_turn_contradicting_a_confirmed_claim_emits_one_knowledge_overridden() {
    let provider = Scripted::start_with_script(turn_with(OVERRIDE_SQL));
    let case =
        knowledge_fixture::build("knowledge-override", &provider.endpoint, Wanted::default()).await;
    let sink = Recording::default();
    let turn = case.turn(&sink, QUESTION).await;
    assert!(turn.is_ok(), "the turn must run: {turn:?}");

    let findings = sink.overridden().expect("one KnowledgeOverridden");
    assert_eq!(
        findings.len(),
        1,
        "exactly one finding, naming the contradicted claim: {:?}",
        sink.events()
    );
    let finding = &findings[0];
    assert_eq!(
        finding.claim_id,
        case.claim_id(),
        "names the supplied confirmed claim"
    );
    assert_eq!(finding.kind, "default_time_column");
    assert_eq!(
        finding.claimed_value, CLAIMED_COLUMN,
        "where you specified Y"
    );
    assert!(
        finding.observed_columns.contains(&OTHER_COLUMN.to_string()),
        "names the column actually referenced: {finding:?}"
    );
}

/// A statement that honours the claim emits nothing.
#[tokio::test]
async fn a_live_turn_honouring_the_claim_emits_no_knowledge_overridden() {
    let provider = Scripted::start_with_script(turn_with(HONOURING_SQL));
    let case = knowledge_fixture::build(
        "knowledge-override-honoured",
        &provider.endpoint,
        Wanted::default(),
    )
    .await;
    let sink = Recording::default();
    let turn = case.turn(&sink, QUESTION).await;
    assert!(turn.is_ok(), "the turn must run: {turn:?}");

    assert!(
        sink.overridden().is_none(),
        "no finding names a claim the SQL honoured: {:?}",
        sink.events()
    );
}

/// Unparseable SQL emits nothing: the detector fails closed rather than guess.
#[tokio::test]
async fn a_live_turn_with_unparseable_sql_emits_no_knowledge_overridden() {
    let provider = Scripted::start_with_script(turn_with(UNPARSEABLE_SQL));
    let case = knowledge_fixture::build(
        "knowledge-override-unparseable",
        &provider.endpoint,
        Wanted::default(),
    )
    .await;
    let sink = Recording::default();
    let turn = case.turn(&sink, QUESTION).await;
    assert!(turn.is_ok(), "the turn must run: {turn:?}");

    assert!(
        sink.overridden().is_none(),
        "unparseable SQL emits no finding: {:?}",
        sink.events()
    );
}

/// A candidate claim is supplied but never overridden: only a confirmed claim
/// binds.
#[tokio::test]
async fn a_live_candidate_claim_contradicted_emits_nothing() {
    let provider = Scripted::start_with_script(turn_with(OVERRIDE_SQL));
    let case = knowledge_fixture::build(
        "knowledge-override-candidate",
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

    // The candidate IS supplied, so the turn says what memory gave it — but
    // only a confirmed claim binds.
    assert_eq!(
        sink.supplied().len(),
        1,
        "the candidate was supplied: {:?}",
        sink.events()
    );
    assert!(
        sink.overridden().is_none(),
        "a candidate claim is not overridable: {:?}",
        sink.events()
    );
}

/// No opaque profile identity leaks into the turn's event stream, the override
/// finding included.
#[tokio::test]
async fn no_identity_leaks_into_the_live_knowledge_overridden_event() {
    let provider = Scripted::start_with_script(turn_with(OVERRIDE_SQL));
    let case = knowledge_fixture::build(
        "knowledge-override-identity",
        &provider.endpoint,
        Wanted::default(),
    )
    .await;
    let identity = case.identity();
    let sink = Recording::default();
    let turn = case.turn(&sink, QUESTION).await;
    assert!(turn.is_ok(), "the turn must run: {turn:?}");

    let stream_json = serde_json::to_string(&sink.events()).unwrap_or_default();
    assert!(
        sink.overridden().is_some(),
        "the turn announced the override the stream is asserted over: {stream_json}"
    );
    assert!(
        !stream_json.contains(identity.as_str()),
        "opaque identity leaked into the event stream: {stream_json}"
    );
}

/// Anti-self-reinforcement, end to end: the model re-infers the very
/// `default_time_column` claim recall supplied, and the post-turn extraction
/// drops it as a duplicate instead of proposing it back.
#[tokio::test]
async fn a_live_turn_does_not_re_ingest_the_claim_it_was_supplied() {
    // The extractor proposes the slot and value recall already supplied.
    let duplicate = format!(
        r#"{{"proposals": [{{"object_id": "T0", "slot": "table.default_time", "value": "{CLAIMED_COLUMN}", "origin": "assistant_inferred"}}]}}"#
    );
    let provider = Scripted::start_with_script(vec![
        Reply::Call {
            name: "bounded_sql_query",
            arguments: json!({ "sql": HONOURING_SQL }),
        },
        Reply::Text("Order dates checked."),
    ]);
    let case = knowledge_fixture::build(
        "knowledge-anti-self-reinforcement",
        &provider.endpoint,
        Wanted {
            extractor: Some(duplicate),
            ..Wanted::default()
        },
    )
    .await;
    let sink = Recording::default();
    let turn = case.turn(&sink, "show me orders").await;
    assert!(turn.is_ok(), "the turn must run: {turn:?}");

    assert!(
        case.extractor.calls() > 0,
        "the post-turn extraction ran: it is what would re-ingest the claim"
    );
    let proposed = sink
        .events()
        .iter()
        .filter(|event| matches!(event, saya_agent::AgentEvent::KnowledgeProposed { .. }))
        .count();
    assert_eq!(
        proposed, 0,
        "anti-self-reinforcement dropped duplicate inference"
    );
}
