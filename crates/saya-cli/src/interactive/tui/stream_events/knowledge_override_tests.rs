//! Tests for the KnowledgeOverridden / KnowledgeLearningSkipped receipt lines, driven through `super::apply_event` (moved verbatim
//! from the inline `tests` module in `stream_events/apply.rs`).

use super::{BlockKind, Transcript, apply_event};
use saya_agent::AgentEvent;
use saya_agent::{LearningSkipReason, OverrideFindingDto};
use saya_types::ClaimId;

fn last_block_text(transcript: &Transcript) -> Option<&str> {
    transcript.blocks().last().map(|b| b.text.as_str())
}

/// KnowledgeOverridden pushes a System block whose text names the referenced
/// column and the specified value. Trails the answer — the
/// block lands below the assistant text in the transcript.
#[test]
fn knowledge_overridden_pushes_a_system_block_naming_the_finding() {
    let mut t = Transcript::new();
    apply_event(
        &mut t,
        AgentEvent::knowledge_overridden(vec![OverrideFindingDto {
            claim_id: ClaimId::parse("c-rental-time").unwrap(),
            kind: "default_time_column".into(),
            claimed_value: "return_date".into(),
            observed_columns: vec!["rental_date".into()],
        }]),
        false,
    );
    let block = last_block_text(&t).expect("a block was pushed");
    assert!(
        block.contains("memory overridden · 1 finding"),
        "header: {block}"
    );
    assert!(
        block.contains("referenced rental_date"),
        "names the referenced column: {block}"
    );
    assert!(
        block.contains("where you specified return_date"),
        "names the specified value: {block}"
    );
    // The wording constraint: no causal "used" about the time column.
    assert!(
        !block.contains("used"),
        "the TUI block must not assert a causal 'used': {block}"
    );
    assert_eq!(t.blocks().last().unwrap().kind, BlockKind::System);
}

/// An empty finding set pushes nothing — silence.
#[test]
fn an_empty_knowledge_overridden_event_pushes_nothing() {
    let mut t = Transcript::new();
    apply_event(&mut t, AgentEvent::knowledge_overridden(Vec::new()), false);
    assert!(
        t.blocks().is_empty(),
        "no findings → no block: {:?}",
        t.blocks()
    );
}

/// KnowledgeLearningSkipped pushes a System block whose text names the skip
/// reason (packet-54 decision 4 — the TUI renders it, it does not fall
/// through to the catch-all that would drop it). Trails the answer.
#[test]
fn knowledge_learning_skipped_pushes_a_system_block_naming_the_reason() {
    let mut t = Transcript::new();
    apply_event(
        &mut t,
        AgentEvent::knowledge_learning_skipped(LearningSkipReason::TimedOut),
        false,
    );
    let block = last_block_text(&t).expect("a block was pushed");
    assert!(
        block.contains("memory not recorded · extraction timed out"),
        "TUI block names the timeout: {block}"
    );
    assert_eq!(t.blocks().last().unwrap().kind, BlockKind::System);

    let mut t = Transcript::new();
    apply_event(
        &mut t,
        AgentEvent::knowledge_learning_skipped(LearningSkipReason::Failed),
        false,
    );
    let block = last_block_text(&t).expect("a block was pushed");
    assert!(
        block.contains("memory not recorded · extraction failed"),
        "TUI block names the failure: {block}"
    );
}
