//! Entry point: clip one table block's lines to the viewport width.
//! Orchestrates geometry (bounds), columns (filter + window), and render
//! (per-line reconstruction). Returns one line per input line.

use crate::interactive::tui::types::WideTableView;

use super::columns::{base_columns, window_columns};
use super::geometry::{clip_chars, line_head, table_bounds};
use super::render::reconstruct_line;

/// Clips one table block's lines to `width` according to the view state.
/// `lines` is the whole block (borders, header, rows, footer). The returned
/// vec has the same length: box lines are reconstructed for the visible
/// columns, the footer passes through. Non-box blocks (no top border) are
/// hard-clipped per line.
pub(crate) fn clip_table_block(lines: &[String], wv: &WideTableView, width: usize) -> Vec<String> {
    let top = lines.iter().position(|l| line_head(l) == '┌');
    let Some(top) = top else {
        return lines.iter().map(|l| clip_chars(l, width)).collect();
    };
    let bottom = lines[top..]
        .iter()
        .rposition(|l| line_head(l) == '└')
        .map(|idx| idx + top);
    let Some(bottom) = bottom else {
        return lines.iter().map(|l| clip_chars(l, width)).collect();
    };

    let bounds = table_bounds(&lines[top..=bottom]);
    if bounds.len() < 2 {
        return lines.iter().map(|l| clip_chars(l, width)).collect();
    }

    let n_cols = bounds.len() - 1;
    let base = base_columns(n_cols, &lines[top + 1], &bounds, wv);
    let visible = window_columns(&base, &bounds, wv, width);
    let at_left = visible.first().is_some_and(|&c| c == base[0]);
    let at_right = visible.last().is_some_and(|&c| c == *base.last().unwrap());

    let mut out = Vec::with_capacity(lines.len());
    for (idx, line) in lines.iter().enumerate() {
        if (top..=bottom).contains(&idx) {
            out.push(reconstruct_line(
                line, &visible, &bounds, at_left, at_right, width,
            ));
        } else {
            // Footer / anything after the box: plain text, not grid-aligned.
            out.push(clip_chars(line, width));
        }
    }
    out
}
