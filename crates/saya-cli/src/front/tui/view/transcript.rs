//! The new TUI's transcript pane: the conversation as the rows a terminal
//! paints, newest turn at the live edge.
//!
//! A pure function of the view model's turns (T1), the facts the surface holds
//! and a width and height. Nothing here reads the terminal, the clock or the
//! model: the caller hands over what it knows, and the pane answers with the
//! rows to paint. What one turn looks like is [`turn`]'s business; this module
//! lays the turns under each other and keeps the live edge.

use ratatui::text::Line;
use std::time::Duration;

use super::turn;
use crate::front::tui::focus::EntryPath;
use crate::front::tui::theme::{Charset, Scheme};
use crate::front::view::TurnView;

/// The facts the pane paints from, besides the turns themselves.
#[derive(Debug, Clone)]
pub(crate) struct TranscriptFacts {
    /// How long each turn took, by its place in the conversation — the folded
    /// summary's last fact. A turn past the end of the list reads as
    /// instantaneous.
    pub(crate) elapsed: Vec<Duration>,
    /// The workspace root a shell step strips its `cd` against, as the caller
    /// abbreviates it.
    pub(crate) workspace: Option<String>,
    /// Which token column paints the rows.
    pub(crate) scheme: Scheme,
    /// Which glyph set the rows draw with.
    pub(crate) charset: Charset,
    /// The entry the block cursor holds, if the surface is in cursor mode: the
    /// one block that carries §6's mark.
    pub(crate) focus: Option<EntryPath>,
    /// Whether the focused entry shows its inspector line (`i`).
    pub(crate) inspector: bool,
    /// Whether the transcript shows every step's full command and the outcome
    /// it reported (`Ctrl+O`), or one compact line per step. The compact
    /// layout is the default: the steps are visible without a key.
    pub(crate) detail: bool,
    /// The frame a step in flight draws its spinner from: one per tick, so a
    /// step being waited on reads as working.
    pub(crate) spinner: usize,
}

impl Default for TranscriptFacts {
    fn default() -> Self {
        Self {
            elapsed: Vec::new(),
            workspace: None,
            scheme: Scheme::Dark,
            charset: Charset::Utf8,
            focus: None,
            inspector: false,
            detail: false,
            spinner: 0,
        }
    }
}

/// The transcript's rows, oldest first, fitted to `width` and `height`.
///
/// The newest turn sits at the bottom: when the conversation is taller than
/// the pane, the last `height` rows are the ones shown, so a streaming answer
/// stays on screen without a scroll offset.
pub(crate) fn rows(
    turns: &[TurnView],
    facts: &TranscriptFacts,
    width: usize,
    height: usize,
) -> Vec<Line<'static>> {
    let mut all: Vec<Line<'static>> = Vec::new();
    for (index, turn) in turns.iter().enumerate() {
        // One blank row between turns: the conversation reads as turns, not as
        // one run of blocks.
        if index > 0 {
            all.push(Line::default());
        }
        let elapsed = facts.elapsed.get(index).copied().unwrap_or_default();
        all.extend(turn::rows(index, turn, elapsed, facts, width));
    }
    let kept = height.min(all.len());
    all.split_off(all.len() - kept)
}
