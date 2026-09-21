//! The cell-aware word wrap shared by the transcript pane and the overlays.
//!
//! One wrapper, many consumers: the transcript flattens its blocks through
//! [`wrap_cells`], and the approval panel measures **and** paints its fact
//! body with the same rows — so what a panel counts is what it draws, and a
//! wide body can never be counted short of its paint (re-audit R01: three
//! wrappers is how that defect happened).
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
