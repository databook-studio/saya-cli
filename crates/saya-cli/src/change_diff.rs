//! The change one file-tool call makes, as the bounded unified diff both
//! surfaces read: the approval panel before the call runs, and the turn's
//! `Changed:` line and changes pane once it has.
//!
//! **One diff, one engine.** The rows come from the checkpoint/rewind preview's
//! own core — `commands::recovery::{line_diff, hunks}`: the line-LCS, the two
//! context lines around a change, the `@@` hunk headers, and the byte, line and
//! cell bounds that keep a hostile input from costing more than a frame. A
//! second diff implementation would be a second set of bounds to keep.
//!
//! **The sides are the call's own bytes.** `edit` states the anchored string it
//! takes away and the one it writes, so those are the before and after sides;
//! `write` states the whole file, so the file it writes is the after side and
//! nothing is taken away. The recorded before-image of a path is written by the
//! tool as it commits, which is after the operator answers — so an approval
//! cannot read it, and this module never pretends to.
//!
//! **Bounded, and said so.** A change over the engine's byte bound, or one that
//! is not text (a NUL byte), is one size row instead of a diff. A diff longer
//! than [`MAX_ROWS`] keeps its first rows and closes with `+K more lines`; the
//! counts are read off the whole change, never off what fits.

use serde_json::Value;

use crate::agent::tools::shell_policy::sanitise_prompt_text;
use crate::commands::recovery::hunks;
use crate::commands::recovery::line_diff::{
    DIFF_MAX_BYTES, DIFF_MAX_CELLS, DIFF_MAX_LINES, Op, lcs_ops, split_lines, text_side,
};

/// The most diff rows one change states: the pane scrolls these, and the
/// approval panel shows its first rows and counts the rest.
pub(crate) const MAX_ROWS: usize = 40;

/// One call's change: the rows a pane paints, and how many lines it adds and
/// takes away over the whole change — never only over the rows that fit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Diff {
    /// The rows: `@@` hunk headers, then the context, `-` and `+` lines; or
    /// one size row for a change too large or not text.
    pub(crate) rows: Vec<String>,
    /// The lines the change writes, over the whole change.
    pub(crate) added: usize,
    /// The lines the change takes away, over the whole change.
    pub(crate) removed: usize,
}

/// The change the call's own arguments state, or `None` for a call that
/// changes no file text.
///
/// The shape is read off the arguments rather than off the tool's name: the
/// consent seam a call passes through carries the caller's line, not the tool,
/// so `path` with `content` is a whole-file write, `old_text`/`new_text` (or
/// `edits`) is an anchored replace, and `offset` with `chunk` is an append. A
/// call stating none of those shapes states no change this module can show, and
/// invents none.
pub(crate) fn of(arguments: &Value) -> Option<Diff> {
    let (before, after) = sides(arguments)?;
    Some(diff_of(&before, &after))
}

/// The two sides of the call's change: what it takes away, and what it writes.
///
/// Both are the call's own bytes. A shape the call does not fully state is
/// `None`: an `edit` with no anchor and no chunk names no change, and so does a
/// `write` with no content.
fn sides(arguments: &Value) -> Option<(String, String)> {
    text(arguments, "path")?;
    if let Some(content) = text(arguments, "content") {
        return Some((String::new(), content));
    }
    if let Some(edits) = arguments.get("edits").and_then(Value::as_array) {
        if edits.is_empty() {
            return None;
        }
        let mut before = Vec::with_capacity(edits.len());
        let mut after = Vec::with_capacity(edits.len());
        for edit in edits {
            before.push(text(edit, "old_text")?);
            after.push(text(edit, "new_text")?);
        }
        return Some((before.join("\n"), after.join("\n")));
    }
    if let Some(old) = named(arguments, &["old_text", "old_string", "oldString"]) {
        let new = named(arguments, &["new_text", "new_string", "newString"])?;
        return Some((old, new));
    }
    if arguments.get("offset").is_some() && arguments.get("chunk").is_some() {
        return Some((String::new(), text(arguments, "chunk")?));
    }
    None
}

/// One string argument, as the call states it: the bytes are checked before
/// anything is sanitised, so a NUL byte still reads as a binary change.
fn text(arguments: &Value, key: &str) -> Option<String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// The first of `keys` the call states: `edit` accepts the camel and snake
/// spellings of its two anchored texts.
fn named(arguments: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| text(arguments, key))
}

/// The bounded diff between the two sides, as the call states them.
fn diff_of(before_raw: &str, after_raw: &str) -> Diff {
    let bytes = before_raw.len().max(after_raw.len());
    // A side over the engine's byte bound, or one that is not text, is stated as
    // its size: a diff of it is not something a pane can paint. The counts still
    // come from the sides, so `Changed:` states the whole change.
    let (raw_before, raw_after) = (split_lines(before_raw).0, split_lines(after_raw).0);
    let stated = |row: String, added: usize, removed: usize| Diff {
        rows: vec![row],
        added,
        removed,
    };
    if bytes > DIFF_MAX_BYTES {
        return stated(
            format!("change: {bytes} bytes · too large to show a diff"),
            raw_after.len(),
            raw_before.len(),
        );
    }
    if text_side(before_raw.as_bytes()).is_none() || text_side(after_raw.as_bytes()).is_none() {
        return stated(
            format!("change: binary · {bytes} bytes · no diff shown"),
            raw_after.len(),
            raw_before.len(),
        );
    }
    // The bytes are text: sanitised before a diff is built, so no control byte
    // reaches a cell of the pane.
    let before = sanitise_prompt_text(before_raw);
    let after = sanitise_prompt_text(after_raw);
    let (before_lines, before_trailing) = split_lines(&before);
    let (after_lines, after_trailing) = split_lines(&after);
    let lines = before_lines.len().max(after_lines.len());
    if before_lines.len() > DIFF_MAX_LINES
        || after_lines.len() > DIFF_MAX_LINES
        || (before_lines.len() + 1).saturating_mul(after_lines.len() + 1) > DIFF_MAX_CELLS
    {
        return stated(
            format!("change: {lines} lines · too large to show a diff"),
            after_lines.len(),
            before_lines.len(),
        );
    }
    if before == after {
        return Diff {
            rows: Vec::new(),
            added: 0,
            removed: 0,
        };
    }
    let ops = lcs_ops(&before_lines, &after_lines);
    let added = ops.iter().filter(|op| matches!(op, Op::Ins(_))).count();
    let removed = ops.iter().filter(|op| matches!(op, Op::Del(_))).count();
    let (rows, _truncated) = hunks::lines_of(
        &before_lines,
        &after_lines,
        &ops,
        before_trailing,
        after_trailing,
    );
    Diff {
        rows: bounded(rows),
        added,
        removed,
    }
}

/// `rows` cut to [`MAX_ROWS`], with the count of what the pane is not showing.
fn bounded(rows: Vec<String>) -> Vec<String> {
    if rows.len() <= MAX_ROWS {
        return rows;
    }
    let withheld = rows.len() - MAX_ROWS;
    let mut kept: Vec<String> = rows.into_iter().take(MAX_ROWS).collect();
    kept.push(format!("+{withheld} more lines"));
    kept
}

#[cfg(test)]
#[path = "change_diff_tests.rs"]
mod tests;
