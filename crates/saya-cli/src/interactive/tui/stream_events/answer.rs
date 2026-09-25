//! Assistant-text and completion arms: delta appends, turn resets, and the
//! `Complete` table reformat.

use super::{BlockKind, Transcript};
use saya_agent::AgentEvent;

use super::super::table;

/// Appends one assistant delta to the answer block, opening it (separated
/// from any tool/SQL lines above) on the first chunk. A retry notice left
/// by `reset_answer` must not capture the re-stream: the emptied assistant
/// block sits *behind* the notice, so the answer resumes into it (found by
/// kind within the chapter in flight, not by the tail) instead of appending
/// to the notice.
/// `append_delta` would extend the trailing System notice; the separator
/// arm below only opens a spacer when the tail is neither the notice nor
/// an assistant block.
pub(crate) fn push_assistant_text(transcript: &mut Transcript, text: &str) {
    if transcript
        .blocks()
        .last()
        .is_some_and(|b| b.kind == BlockKind::System && b.text == RETRY_NOTICE)
        && live_answer_index(transcript).is_some()
    {
        transcript.reformat_last(BlockKind::Assistant, |current| format!("{current}{text}"));
        return;
    }
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

/// The retry notice, worded exactly as the piped surface renders
/// `TerminalEvent::TurnReset` (`render/mod.rs`). A literal copy, not a
/// shared constant: `render/mod.rs` is outside this packet's owned paths,
/// so the parity test `the_retry_notice_matches_the_piped_surface` guards
/// the wording instead — it derives the expected string from the piped
/// adapter at runtime and fails on any drift.
pub(crate) const RETRY_NOTICE: &str = "provider stream interrupted — retrying";

/// The index of the live assistant answer in the chapter in flight, if any:
/// the last `Assistant` block carrying the tail block's chapter. The reverse
/// scan is bounded to that chapter — the same identity `current_chapter`
/// derives (`transcript/chapters/mod.rs`) — so a completed answer from an
/// earlier chapter is never reachable from here: losing delivered evidence
/// is worse than any duplicate, and a stream that died before answering has
/// nothing of its own to find. Chapters are contiguous and only advance, so
/// the first kind-match scanning backward is the chapter's own answer.
fn live_answer_index(transcript: &Transcript) -> Option<usize> {
    let blocks = transcript.blocks();
    let chapter = blocks.last()?.chapter;
    let index = blocks
        .iter()
        .rposition(|block| block.kind == BlockKind::Assistant && block.chapter == chapter)?;
    // Chapter is not attempt. When an attempt boundary is known, an answer
    // block that predates it belongs to an earlier step of this request and
    // is not the live answer to resume into — appending the retried text to
    // it overwrites delivered evidence (re-audit R02). Without a mark the
    // chapter bound stands, which is slice B's behaviour.
    match transcript.attempt_start() {
        Some(start) if index < start => None,
        _ => Some(index),
    }
}

/// The provider stream failed mid-answer and the loop is retrying the
/// turn. The text streamed so far is discarded: the live assistant block
/// is cleared, and a `System` notice is pushed after it. The ordering is
/// the whole trick: clearing first means the notice (a System block, not
/// Assistant) is never itself cleared by this or a later reset, and the
/// re-streamed answer resumes into the emptied assistant block *behind*
/// the notice (by kind within the chapter in `push_assistant_text`), so it
/// neither appends to the notice nor opens a second assistant block. A
/// reset with nothing streamed says nothing: there is no discarded attempt
/// to announce, and the next answer still opens its own block.
pub(crate) fn reset_answer(transcript: &mut Transcript) {
    // The discarded attempt is the live answer of the chapter in flight:
    // either the trailing assistant block (mid-answer) or the resumed answer
    // behind a trailing notice (a second retry landing after the re-stream
    // began). No answer in this chapter — the stream died before it wrote
    // anything — means nothing to discard and nothing to clear: the search
    // must not walk past the chapter boundary to find something to erase.
    let Some(index) = live_answer_index(transcript) else {
        return;
    };
    let discarded = !transcript.blocks()[index].text.is_empty();
    // Clear the live answer wherever it sits — trailing, or behind a
    // trailing notice — so the next delta resumes it instead of appending
    // to the partial. `reset_delta` only clears the trailing block, so a
    // notice-trailing retry needs `reformat_last` to reach behind it; the
    // notice itself (System, never Assistant) is untouched either way. The
    // index above guarantees the kind-match `reformat_last` finds is this
    // chapter's own answer.
    if transcript
        .blocks()
        .last()
        .is_some_and(|block| block.kind == BlockKind::Assistant)
    {
        transcript.reset_delta(BlockKind::Assistant);
    } else {
        transcript.reformat_last(BlockKind::Assistant, |_| String::new());
    }
    if discarded {
        transcript.push(BlockKind::System, RETRY_NOTICE);
    }
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
