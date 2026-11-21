//! The blocks `02` §3.4-§3.9 draws beside the message, the activity group, the
//! answer and the result preview: an edit and its diff, a command and how it
//! ended, a helper's work, an error, a stop and the model's clarifying
//! question.
//!
//! Presentation-free like its siblings — no glyph, no colour, no width — so the
//! pane above decides how a block is painted and a test can assert the facts it
//! holds without a terminal.

use std::time::Duration;

use saya_agent::BudgetDimension;

/// An edit of one file and the diff of it (`02` §3.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Edit {
    /// The file, as the call named it.
    pub(crate) path: String,
    /// The diff's rows, in file order.
    pub(crate) lines: Vec<DiffLine>,
}

impl Edit {
    /// How many lines the edit adds.
    pub(crate) fn added(&self) -> usize {
        self.count(DiffKind::Added)
    }

    /// How many lines the edit removes.
    pub(crate) fn removed(&self) -> usize {
        self.count(DiffKind::Removed)
    }

    /// How many rows of `kind` the diff holds.
    fn count(&self, kind: DiffKind) -> usize {
        self.lines.iter().filter(|line| line.kind == kind).count()
    }
}

/// One row of a diff: its line in the file, where it stands, and the line
/// itself, indentation and all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiffLine {
    /// The line's number in the file, when the caller knows it — a call's own
    /// arguments name the text, not where it sat.
    pub(crate) number: Option<u32>,
    pub(crate) kind: DiffKind,
    /// The line as the file holds it.
    pub(crate) text: String,
}

impl DiffLine {
    /// A row that reads on both sides of the diff.
    pub(crate) fn context(number: u32, text: &str) -> Self {
        Self::at(number, DiffKind::Context, text)
    }

    /// A row the edit writes.
    pub(crate) fn added(number: u32, text: &str) -> Self {
        Self::at(number, DiffKind::Added, text)
    }

    /// A row the edit takes away.
    pub(crate) fn removed(number: u32, text: &str) -> Self {
        Self::at(number, DiffKind::Removed, text)
    }

    /// A row at `number`.
    fn at(number: u32, kind: DiffKind, text: &str) -> Self {
        Self {
            number: Some(number),
            kind,
            text: text.to_owned(),
        }
    }
}

/// Where one diff row stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiffKind {
    /// Unchanged, shown for context.
    Context,
    /// Written by the edit.
    Added,
    /// Taken away by the edit.
    Removed,
}

/// A command the turn ran, and how it ended (`02` §3.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Command {
    /// The command as the call wrote it; the pane strips a `cd` into the
    /// workspace, so this may name the root.
    pub(crate) command: String,
    pub(crate) state: CommandState,
}

/// How a command ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CommandState {
    /// It exited, and took this long.
    Finished { elapsed: Duration },
    /// It exited non-zero, in its own words, and took this long.
    Failed { reason: String, elapsed: Duration },
}

/// A helper's work, as one block (`02` §3.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Helper {
    /// The helper's id, as the call named it.
    pub(crate) id: String,
    /// The helper's brief, in its own words.
    pub(crate) question: String,
    pub(crate) steps: usize,
    pub(crate) state: HelperState,
}

/// How a helper's work stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HelperState {
    Running,
    Done,
    Failed,
}

/// Something that ended the turn in failure (`02` §3.7).
///
/// Never folded and never merged into the activity group: the message is the
/// reason, and the reason is the part the operator has to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Error {
    /// What failed, in the words of whoever stopped it.
    pub(crate) message: String,
}

/// What stopped the turn, or the operator's cancel (`02` §3.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Stop {
    /// The turn's own budget ran out before it answered in full.
    Budget {
        dimension: BudgetDimension,
        /// The key and value that bound the turn, in the caller's words, when
        /// the surface knows them.
        limit: Option<String>,
    },
    /// A guard paused the turn; the message is the guard's own sentence.
    Paused { message: String },
    /// The operator cancelled the turn. The run stays open.
    Cancelled,
}

/// The model's clarifying question and the answers it offered (`02` §3.9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Clarification {
    /// The question, in the model's own words.
    pub(crate) question: String,
    /// The candidate answers it offered, in the order it offered them; empty
    /// when it offered none.
    pub(crate) options: Vec<String>,
}
