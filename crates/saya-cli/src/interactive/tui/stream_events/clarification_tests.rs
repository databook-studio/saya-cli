//! Tests for the clarification turn in the TUI transcript (B3c): a
//! `ClarificationNeeded` event flushes the buffered tool group and pushes a
//! distinct Question block with the options numbered — the user's next
//! message is ordinary input, never a modal.

use super::{BlockKind, Transcript, apply_event};
use saya_agent::{AgentEvent, LocalStateEffect, ToolEffect};

fn ask_event() -> AgentEvent {
    AgentEvent::clarification_needed(
        "Which metric should \"active users\" use?",
        vec![
            "sessions in the last 30 days".into(),
            "purchases in the last 90 days".into(),
        ],
    )
}

fn none_effect() -> ToolEffect {
    ToolEffect {
        database_data: false,
        external_side_effect: false,
        requires_approval: false,
        local_state: LocalStateEffect::None,
    }
}

/// The question block is distinct (a System block naming "question"), carries
/// the question text, and numbers the options.
#[test]
fn a_clarification_renders_a_distinct_question_block_with_numbered_options() {
    let mut t = Transcript::new();
    apply_event(
        &mut t,
        AgentEvent::tool_requested(
            "request_clarification",
            serde_json::json!({"question": "Which metric should \"active users\" use?"}),
            Some(none_effect()),
        ),
        false,
    );
    apply_event(
        &mut t,
        AgentEvent::ToolCompleted {
            name: "request_clarification".into(),
            summary: "question asked — the turn pauses for the user's answer".into(),
        },
        false,
    );
    apply_event(&mut t, ask_event(), false);
    let block = t.blocks().last().expect("the question block was pushed");
    assert_eq!(
        block.kind,
        BlockKind::System,
        "the question is its own block"
    );
    assert!(
        block.text.contains("question"),
        "the block names itself a question: {:?}",
        block.text
    );
    assert!(
        block
            .text
            .contains("Which metric should \"active users\" use?"),
        "the question text is carried: {:?}",
        block.text
    );
    assert!(
        block.text.contains("1. sessions in the last 30 days"),
        "options are numbered: {:?}",
        block.text
    );
    assert!(
        block.text.contains("2. purchases in the last 90 days"),
        "options are numbered: {:?}",
        block.text
    );
    // The buffered tool group flushed before the question: the activity is on
    // screen above it.
    assert!(
        t.blocks()
            .iter()
            .any(|block| block.kind == BlockKind::Tool
                && block.text.contains("request_clarification")),
        "the buffered ask renders before the question block: {:?}",
        t.blocks()
            .iter()
            .map(|b| (b.kind, &b.text))
            .collect::<Vec<_>>()
    );
}

/// An ask without options renders the question alone — no numbered lines, no
/// empty option block.
#[test]
fn an_ask_without_options_renders_the_question_alone() {
    let mut t = Transcript::new();
    apply_event(
        &mut t,
        AgentEvent::clarification_needed("Which table holds active users?", Vec::new()),
        false,
    );
    let block = t.blocks().last().expect("the question block was pushed");
    assert!(
        block.text.contains("Which table holds active users?"),
        "{block:?}"
    );
    assert!(
        !block.text.contains("1."),
        "no options, no numbering: {block:?}"
    );
}

/// The question block is an ordinary System block: the user's next message
/// opens a fresh User block after it — nothing special to answer through.
#[test]
fn the_next_user_message_is_ordinary_input_after_the_question() {
    let mut t = Transcript::new();
    apply_event(&mut t, ask_event(), false);
    apply_event(&mut t, AgentEvent::complete(), false);
    // Simulate the user's answer: a fresh User block follows the question.
    t.push(BlockKind::User, "sessions in the last 30 days");
    let blocks = t.blocks();
    let question_index = blocks
        .iter()
        .position(|block| block.text.contains("Which metric"))
        .expect("the question block");
    assert!(
        matches!(blocks.get(question_index + 1), Some(block) if block.kind == BlockKind::User),
        "the answer is an ordinary user turn: {:?}",
        blocks.iter().map(|b| (b.kind, &b.text)).collect::<Vec<_>>()
    );
}
