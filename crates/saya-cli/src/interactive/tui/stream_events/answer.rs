//! Assistant-text and completion arms: delta appends, turn resets, and the
//! `Complete` table reformat.

use super::{BlockKind, Transcript};
use saya_agent::AgentEvent;

use super::super::table;

/// Appends one assistant delta to the answer block, opening it (separated
/// from any tool/SQL lines above) on the first chunk.
pub(crate) fn push_assistant_text(transcript: &mut Transcript, text: &str) {
    if !matches!(
        transcript.blocks().last().map(|b| b.kind),
        Some(BlockKind::Assistant)
    ) {
        // first chunk of the answer: separate it from the tool/SQL lines above
        if transcript
            .blocks()
            .last()
            .is_some_and(|b| !b.text.is_empty())
        {
            transcript.push(BlockKind::System, String::new());
        }
    }
    transcript.append_delta(BlockKind::Assistant, text);
}

/// The provider stream failed mid-answer and the loop is retrying the
/// turn. The text streamed so far is discarded: clear the trailing
/// assistant block (the one the next delta would extend) so the
/// re-streamed answer replaces it instead of appending to it.
pub(crate) fn reset_answer(transcript: &mut Transcript) {
    transcript.reset_delta(BlockKind::Assistant);
}

/// The turn is done: reformat the trailing answer block's markdown tables.
pub(crate) fn finish_answer(transcript: &mut Transcript) {
    transcript.reformat_last(BlockKind::Assistant, table::format_markdown_tables);
}

/// Whether the event is an assistant-text chunk, a turn reset, or completion.
#[allow(dead_code)]
pub(crate) fn is_answer_event(event: &AgentEvent) -> bool {
    matches!(
        event,
        AgentEvent::AssistantText { .. } | AgentEvent::TurnReset | AgentEvent::Complete
    )
}

/// Dispatches one answer-lifecycle event. Returns true when handled.
pub(crate) fn apply_answer_event(transcript: &mut Transcript, event: AgentEvent) -> bool {
    match event {
        AgentEvent::AssistantText { text } => {
            push_assistant_text(transcript, &text);
            true
        }
        // The retry discards the run in flight: the failure it reports belongs to
        // the transport, never to a collapsed summary.
        AgentEvent::TurnReset => {
            reset_answer(transcript);
            true
        }
        AgentEvent::Complete => {
            finish_answer(transcript);
            true
        }
        _ => false,
    }
}
