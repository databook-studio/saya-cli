//! What a turn changed, as the transcript's `Changed:` line and the pane behind
//! it state it (`02` §3.4, `06` §5): one entry per file, the diff rows the pane
//! scrolls, and the counts the line reads.
//!
//! Presentation-free like its siblings — no glyph, no colour, no width — so a
//! test asserts the words without a terminal. The diff itself is
//! [`crate::change_diff`]'s: one engine, one set of bounds, whether the change is
//! being approved or looked back at.

use serde_json::Value;

use super::entry::{Entry, Step, StepState, TurnView};
use crate::change_diff::{self, Diff};

/// The key that opens the changes pane, as the line and the sheet write it.
pub(crate) const KEY: &str = "Ctrl+D";

/// The most files one `Changed:` line names; a longer turn says how many it
/// left out rather than growing a row the width cannot hold.
pub(crate) const MAX_FILES: usize = 4;

/// One file a turn changed: its path, the rows of its diff, and both counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Change {
    /// The file, as the call named it.
    pub(crate) path: String,
    /// The diff's rows, bounded by [`crate::change_diff::MAX_ROWS`].
    pub(crate) rows: Vec<String>,
    /// The lines the turn wrote to it.
    pub(crate) added: usize,
    /// The lines the turn took away from it.
    pub(crate) removed: usize,
}

/// The files `turn` changed, in the order its calls first named them; two calls
/// to one file are one entry, because the operator reads a file, not a call.
pub(crate) fn changed(turn: &TurnView) -> Vec<Change> {
    let mut files: Vec<Change> = Vec::new();
    let Some(activity) = turn.entries().iter().find_map(Entry::activity) else {
        return files;
    };
    for step in &activity.steps {
        let Some(change) = step_change(step) else {
            continue;
        };
        match files.iter_mut().find(|held| held.path == change.path) {
            Some(held) => {
                held.rows.extend(change.rows);
                held.added += change.added;
                held.removed += change.removed;
            }
            None => files.push(change),
        }
    }
    files
}

/// The files the newest turn that changed any changed: what the changes pane
/// opens on, so a turn that only read files never hides the last one that wrote.
pub(crate) fn last_changed(turns: &[TurnView]) -> Vec<Change> {
    for turn in turns.iter().rev() {
        let files = changed(turn);
        if !files.is_empty() {
            return files;
        }
    }
    Vec::new()
}

/// The `Changed:` line for `files`, or `None` when they changed nothing.
pub(crate) fn summary(files: &[Change]) -> Option<String> {
    if files.is_empty() {
        return None;
    }
    let mut named: Vec<String> = files
        .iter()
        .take(MAX_FILES)
        .map(|file| format!("{} (+{} −{})", file.path, file.added, file.removed))
        .collect();
    let withheld = files.len().saturating_sub(MAX_FILES);
    if withheld > 0 {
        named.push(format!("+{withheld} more files"));
    }
    Some(format!("Changed: {} · {KEY} diff", named.join(", ")))
}

/// One step's change, when the step is a file call that landed.
///
/// A refused, denied or still-running call changed nothing, and a call whose
/// change cannot be stated from its own bytes is left off rather than guessed
/// at: the line counts what the turn did.
fn step_change(step: &Step) -> Option<Change> {
    if !matches!(step.state, StepState::Done { .. }) {
        return None;
    }
    let Diff {
        rows,
        added,
        removed,
    } = change_diff::of(&step.arguments)?;
    if rows.is_empty() {
        return None;
    }
    let path = step.arguments.get("path").and_then(Value::as_str)?;
    Some(Change {
        path: crate::agent::tools::collapse_whitespace(path),
        rows,
        added,
        removed,
    })
}
