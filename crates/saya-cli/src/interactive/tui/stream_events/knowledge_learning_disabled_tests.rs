//! Tests for the KnowledgeLearningDisabled breaker notice, driven through
//! `super::apply_event` the same way `knowledge_supplied_tests.rs` does.

use super::{BlockKind, Transcript, apply_event};
use saya_agent::AgentEvent;

fn last_block_text(transcript: &Transcript) -> Option<&str> {
    transcript.blocks().last().map(|b| b.text.as_str())
}

/// KnowledgeLearningDisabled pushes a System block carrying the spec line,
/// naming the model and the miss count — the TUI half of red test 7.
#[test]
fn knowledge_learning_disabled_pushes_a_system_block_with_the_spec_line() {
    let mut t = Transcript::new();
    apply_event(
        &mut t,
        AgentEvent::knowledge_learning_disabled("glm-5.2", 2),
        false,
    );
    let block = last_block_text(&t).expect("a block was pushed");
    assert_eq!(
        block,
        "memory: learning disabled for this session — extraction with glm-5.2 was cut off or \
         stalled 2 times in a row. Recall still works; /remember still stores a rule."
    );
    assert_eq!(t.blocks().last().unwrap().kind, BlockKind::System);
}
