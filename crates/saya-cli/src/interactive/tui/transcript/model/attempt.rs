//! The rollback watermark for one provider attempt.
//!
//! `AgentEvent::TurnReset` means "discard what *this attempt* produced", and
//! before slice O the TUI had no way to say where the attempt began. It used
//! the request chapter instead, which spans the whole request, so a receive
//! that died before emitting text erased an earlier step's delivered preamble
//! and threw away completed tool results (re-audit R02). A chapter is a
//! presentation decision; an attempt is a lifecycle fact, and they are not
//! the same boundary.
//!
//! `AgentEvent::TurnStarted` now arrives before every attempt, first and
//! retried alike. The mark taken there is everything needed to undo exactly
//! what follows it and nothing earlier.

use super::super::{BlockKind, Transcript};
use super::PendingToolCall;

/// Where one attempt began: enough state to truncate back to it.
///
/// `pending_tools` is snapshotted whole rather than by length, because a
/// completion *mutates an existing entry* — setting its summary, closing it,
/// and growing its `live_blocks`. Restoring only the length would leave a
/// call that was requested before the mark carrying a result from the
/// attempt that just failed.
#[derive(Debug, Clone)]
pub(crate) struct AttemptMark {
    /// Block count at the attempt's start; everything at or past this index
    /// was produced by the attempt.
    blocks: usize,
    /// Byte length of the trailing block's text when that block is an
    /// assistant answer, so a partial delta appended by the attempt is
    /// truncated rather than the whole answer cleared. `None` when the tail
    /// is not an assistant block.
    tail_assistant_len: Option<usize>,
    /// The buffered tool run exactly as it stood.
    pending_tools: Vec<PendingToolCall>,
}

impl Transcript {
    /// The block index the attempt in flight began at, when one has been
    /// marked. The resume path uses it to refuse an answer block that
    /// predates the attempt: after a rollback the tail is the retry notice,
    /// and the newest assistant block *in the chapter* is the earlier step's
    /// preamble — appending the replacement to it is how R02's "reuse that
    /// older answer position" happens.
    pub(crate) fn attempt_start(&self) -> Option<usize> {
        self.attempt.as_ref().map(|mark| mark.blocks)
    }

    /// Records where the attempt now beginning starts. Called for every
    /// `TurnStarted`, so the mark always names the attempt in flight.
    pub(crate) fn mark_attempt(&mut self) {
        let tail_assistant_len = self
            .blocks
            .last()
            .filter(|block| block.kind == BlockKind::Assistant)
            .map(|block| block.text.len());
        self.attempt = Some(AttemptMark {
            blocks: self.blocks.len(),
            tail_assistant_len,
            pending_tools: self.pending_tools.clone(),
        });
    }

    /// Rolls back to the current attempt's mark: drops blocks the attempt
    /// appended, truncates the partial answer it streamed into a block that
    /// predates it, and restores the buffered tool run.
    ///
    /// Everything delivered before the mark survives — that is the whole
    /// point. With no mark this does nothing rather than guess; a reset
    /// before any `TurnStarted` is not a shape the producer emits, and
    /// clearing on a hunch is what destroyed evidence before.
    ///
    /// Routed through [`Transcript::mutate`] so the shrink keeps a scrolled
    /// reader where they were (slice N).
    pub(crate) fn rollback_attempt(&mut self) -> bool {
        let Some(mark) = self.attempt.clone() else {
            return false;
        };
        // Did this attempt stream any answer text? Only then is there a
        // discarded attempt to announce — slice B's rule, kept: a reset with
        // nothing streamed says nothing.
        let discarded = self.blocks[mark.blocks.min(self.blocks.len())..]
            .iter()
            .any(|block| block.kind == BlockKind::Assistant && !block.text.is_empty())
            || mark
                .tail_assistant_len
                .zip(self.blocks.get(mark.blocks.saturating_sub(1)))
                .is_some_and(|(len, block)| {
                    block.kind == BlockKind::Assistant && block.text.len() > len
                });
        self.mutate(|t| {
            t.blocks.truncate(mark.blocks);
            if let Some(len) = mark.tail_assistant_len
                && let Some(block) = t.blocks.last_mut()
                && block.kind == BlockKind::Assistant
            {
                block.text.truncate(len);
            }
            t.pending_tools = mark.pending_tools;
        });
        if discarded {
            // Its own mutation, appended after the truncation, so the notice
            // is never itself rolled back and the scrolled anchor is
            // compensated for both changes.
            self.push(
                BlockKind::System,
                crate::interactive::tui::stream_events::answer::RETRY_NOTICE,
            );
        }
        true
    }
}
