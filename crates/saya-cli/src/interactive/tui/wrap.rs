//! The cell-aware word wrap shared by the transcript pane and the overlays.
//!
//! One wrapper, many consumers: the transcript flattens its blocks through
//! [`wrap_cells`], and the approval panel measures **and** paints its fact
//! body with the same rows — so what a panel counts is what it draws, and a
//! wide body can never be counted short of its paint (re-audit R01: three
//! wrappers is how that defect happened). The budgets that truncate instead
//! of wrap — the busy bar's action and its detail head, the folded chapter
//! summary — cut through [`truncate_cells`] and measure through
//! [`cell_width`], so no site rediscovers the units.
//!
//! The budget is measured in terminal cells (`unicode_width`), not scalar
//! values: `日` is one `char` but two cells, a combining mark is several
//! `char`s but no cell. Breaks land on grapheme boundaries
//! (`unicode_segmentation`), so a base never loses its accent and a ZWJ
//! emoji sequence is never cut in half. A single grapheme wider than the
//! whole budget is emitted alone and the loop advances — every iteration
//! consumes at least one grapheme, so wide and zero-width content cannot
//! spin. Control characters (`width() == None`) cost one cell, keeping the
//! pure-ASCII break points identical to a scalar-count loop.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Wraps one logical line to `width` display cells, preferring the last
/// whitespace inside the cell window so words are not split mid-word;
/// over-long single tokens still split (they have nowhere else to go). An
/// empty input yields one empty row — a blank logical line still paints.
pub(crate) fn wrap_cells(raw: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let graphemes: Vec<&str> = raw.graphemes(true).collect();
    let widths: Vec<usize> = graphemes.iter().map(|grapheme| grapheme.width()).collect();
    let mut rows: Vec<String> = Vec::new();
    let mut start = 0;
    while start < graphemes.len() {
        let mut end = start;
        let mut used = 0;
        while end < graphemes.len() && used + widths[end] <= width {
            used += widths[end];
            end += 1;
        }
        if end == graphemes.len() {
            rows.push(graphemes[start..].concat());
            break;
        }
        let (emit_end, next_start) = if end > start {
            // Last whitespace grapheme in the cell window (never the first,
            // or we would loop).
            let space = graphemes[start..end]
                .iter()
                .rposition(|grapheme| grapheme.chars().all(char::is_whitespace))
                .filter(|&index| index > 0);
            match space {
                // Break on the whitespace: it ends this line (trimmed) and
                // is skipped.
                Some(index) => (index, index + 1),
                None => (end - start, end - start),
            }
        } else {
            // One grapheme wider than the whole budget: emit it alone and
            // advance, or the loop would spin forever.
            (1, 1)
        };
        rows.push(graphemes[start..start + emit_end].concat());
        start += next_start;
    }
    if rows.is_empty() {
        rows.push(String::new());
    }
    rows
}

/// A string's cost in terminal cells — the unit every column budget in the
/// TUI is counted in. A `char` is not a cell (`日` is one char, two cells; a
/// combining mark is two chars, no cell), so a site that compares against a
/// `width` measures with this and never with `chars().count()`.
pub(crate) fn cell_width(text: &str) -> usize {
    text.width()
}

/// Cuts `raw` to at most `width` display cells, on a grapheme boundary — the
/// single-line companion to [`wrap_cells`] for the budgets that truncate
/// rather than wrap. The cut stops before the grapheme that would exceed the
/// budget, so a base never loses its combining mark, a ZWJ sequence is never
/// split, and the result never overruns; a lone grapheme wider than the
/// whole budget yields an empty cut rather than an exception to that, and
/// the loop advances without spinning. An ellipsis the caller appends is the
/// caller's one cell to reserve out of `width`.
pub(crate) fn truncate_cells(raw: &str, width: usize) -> String {
    let mut cut = String::new();
    let mut used = 0;
    for grapheme in raw.graphemes(true) {
        let cells = grapheme.width();
        if used + cells > width {
            break;
        }
        used += cells;
        cut.push_str(grapheme);
    }
    cut
}
