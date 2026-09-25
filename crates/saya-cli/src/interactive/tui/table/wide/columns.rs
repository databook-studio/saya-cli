//! Column selection: the `/columns` name filter (`base_columns`) and the
//! viewport window with pin-first and horizontal offset (`window_columns`).

use crate::interactive::tui::types::WideTableView;

use super::geometry::cell_text;

/// Column indices to consider, after applying the `/columns` name filter.
/// An empty filter result falls back to all columns so the view never shows an
/// empty table.
pub(super) fn base_columns(
    n_cols: usize,
    header: &str,
    bounds: &[usize],
    wv: &WideTableView,
) -> Vec<usize> {
    let all: Vec<usize> = (0..n_cols).collect();
    let Some(names) = wv.columns.as_deref() else {
        return all;
    };
    let wanted: Vec<String> = names.iter().map(|n| n.to_lowercase()).collect();
    let header_chars: Vec<char> = header.chars().collect();
    let selected: Vec<usize> = all
        .iter()
        .filter(|&&i| {
            let name = cell_text(&header_chars, bounds, i).to_lowercase();
            wanted.contains(&name)
        })
        .copied()
        .collect();
    if selected.is_empty() { all } else { selected }
}

/// The columns that fit the viewport, applying pin-first and the horizontal
/// offset. Column widths come from the boundary gaps, and each shown column
/// costs its content plus one `│` separator.
pub(super) fn window_columns(
    base: &[usize],
    bounds: &[usize],
    wv: &WideTableView,
    width: usize,
) -> Vec<usize> {
    if base.is_empty() || width == 0 {
        return Vec::new();
    }
    let content_w = |col: usize| {
        bounds[col + 1]
            .saturating_sub(bounds[col])
            .saturating_sub(1)
    };
    let mut shown: Vec<usize> = Vec::new();
    let mut budget = width.saturating_sub(1); // leading │

    if wv.pin_first {
        let c0 = base[0];
        shown.push(c0);
        budget = budget.saturating_sub(content_w(c0) + 1);
    }

    let start = if wv.pin_first {
        wv.h_offset.max(1)
    } else {
        wv.h_offset
    };
    let mut k = start.min(base.len() - 1);
    if shown.is_empty() {
        shown.push(base[k]);
        budget = budget.saturating_sub(content_w(base[k]) + 1);
        k += 1;
    }
    while k < base.len() {
        let cost = content_w(base[k]) + 1;
        if cost > budget {
            break;
        }
        shown.push(base[k]);
        budget -= cost;
        k += 1;
    }
    shown
}
