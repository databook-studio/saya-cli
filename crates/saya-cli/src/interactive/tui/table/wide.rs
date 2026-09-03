//! Horizontal clipping for wide result tables. The transcript stores a table
//! block as its full, box-drawn text (what copy and persistence read); this
//! module paints a horizontally-scrolled, optionally column-filtered window of
//! it to the viewport width. The view owns the offset and column state — this
//! code only reads it.

use crate::interactive::tui::types::WideTableView;

/// Box-drawing chars that sit on a column boundary. They occur at the same
/// character columns in every line of one table, so any border or row line
/// yields the same boundary set.
fn is_boundary(ch: char) -> bool {
    matches!(
        ch,
        '│' | '┌' | '┐' | '└' | '┘' | '├' | '┤' | '┬' | '┴' | '┼'
    )
}

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

fn line_head(line: &str) -> char {
    line.chars().next().unwrap_or(' ')
}

/// Boundary character columns, taken from the separator line when present (it
/// never carries cell content) and falling back to the header line.
fn table_bounds(box_lines: &[String]) -> Vec<usize> {
    let sep = box_lines.iter().find(|l| line_head(l) == '├');
    let source = sep.or_else(|| box_lines.iter().find(|l| line_head(l) == '│'));
    let Some(source) = source else {
        return Vec::new();
    };
    let mut bounds: Vec<usize> = source
        .chars()
        .enumerate()
        .filter(|(_, c)| is_boundary(*c))
        .map(|(i, _)| i)
        .collect();
    bounds.dedup();
    bounds
}

/// Column indices to consider, after applying the `/columns` name filter.
/// An empty filter result falls back to all columns so the view never shows an
/// empty table.
fn base_columns(n_cols: usize, header: &str, bounds: &[usize], wv: &WideTableView) -> Vec<usize> {
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
fn window_columns(
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

fn cell_text(chars: &[char], bounds: &[usize], col: usize) -> String {
    let start = bounds[col] + 1;
    let end = bounds[col + 1];
    let s: String = chars[start..end].iter().collect();
    s.trim().to_string()
}

/// Rebuilds one box line for the visible columns. Rows use plain `│` edges;
/// borders use open edges (`├`/`┤`) when the window does not start at the first
/// or end at the last column, signalling that more columns lie off-screen.
fn reconstruct_line(
    line: &str,
    visible: &[usize],
    bounds: &[usize],
    at_left: bool,
    at_right: bool,
    width: usize,
) -> String {
    let chars: Vec<char> = line.chars().collect();
    let head = chars.first().copied().unwrap_or(' ');
    let segment =
        |col: usize| -> String { chars[bounds[col] + 1..bounds[col + 1]].iter().collect() };

    let is_border = matches!(head, '┌' | '├' | '└');
    if is_border {
        let (left_start, left_cut, junction, right_end, right_cut) = match head {
            '┌' => ('┌', '├', '┬', '┐', '┤'),
            '├' => ('├', '┼', '┼', '┤', '┼'),
            '└' => ('└', '├', '┴', '┘', '┤'),
            _ => unreachable!(),
        };
        let left = if at_left { left_start } else { left_cut };
        let right = if at_right { right_end } else { right_cut };
        let mut s = String::new();
        s.push(left);
        for (pos, &col) in visible.iter().enumerate() {
            if pos > 0 {
                s.push(junction);
            }
            s.push_str(&segment(col));
        }
        s.push(right);
        clip_chars(&s, width)
    } else {
        let mut s = String::from('│');
        for &col in visible {
            s.push_str(&segment(col));
            s.push('│');
        }
        clip_chars(&s, width)
    }
}

fn clip_chars(s: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    s.chars().take(width).collect()
}

/// Header names of a box table block, in column order, or `None` if the text is
/// not a box table. Used by `/columns` to report what is available.
pub(crate) fn column_names(block_text: &str) -> Option<Vec<String>> {
    let lines: Vec<&str> = block_text.lines().collect();
    let top = lines.iter().position(|l| line_head(l) == '┌')?;
    let bottom = lines[top..].iter().rposition(|l| line_head(l) == '└')? + top;
    if bottom <= top + 1 {
        return None;
    }
    let bounds = table_bounds(
        &lines[top..=bottom]
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>(),
    );
    if bounds.len() < 2 {
        return None;
    }
    let header: Vec<char> = lines[top + 1].chars().collect();
    let names = (0..bounds.len() - 1)
        .map(|i| cell_text(&header, &bounds, i))
        .collect();
    Some(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interactive::tui::table::format_table;
    use saya_types::QueryResult;

    fn wide_result() -> QueryResult {
        QueryResult {
            columns: vec![
                "id".into(),
                "name".into(),
                "status".into(),
                "region".into(),
                "country".into(),
                "city".into(),
                "postal".into(),
                "carrier".into(),
                "tracking".into(),
                "total".into(),
                "tax".into(),
                "discount".into(),
                "currency".into(),
                "placed_at".into(),
                "shipped_at".into(),
            ],
            rows: vec![serde_json::json!([
                1,
                "alice",
                "fulfilled",
                "north",
                "CA",
                "SF",
                "94105",
                "UPS",
                "1Z999",
                120,
                12,
                5,
                "USD",
                "2024-01-01",
                "2024-01-03"
            ])],
            row_count: 1,
            truncated: false,
            executed_sql: "SELECT * FROM orders".into(),
        }
    }

    fn block_lines(result: &QueryResult) -> Vec<String> {
        format_table(result).lines().map(str::to_string).collect()
    }

    fn view(h_offset: usize, pin_first: bool, columns: Option<Vec<&str>>) -> WideTableView {
        WideTableView {
            h_offset,
            pin_first,
            columns: columns.map(|c| c.into_iter().map(String::from).collect()),
        }
    }

    #[test]
    fn a_narrow_view_shows_the_first_columns_and_the_footer() {
        let lines = block_lines(&wide_result());
        let wv = view(0, false, None);
        let out = clip_table_block(&lines, &wv, 20);
        // The footer line is preserved verbatim.
        assert!(
            out.iter().any(|l| l.contains("1 row(s)")),
            "footer must survive clipping: {out:?}"
        );
        // Column 0 (id) is visible at the left; a later column is not.
        let header = out
            .iter()
            .find(|l| l.starts_with('│') && l.contains("id"))
            .expect("header row present");
        assert!(header.contains("id"), "first column shows: {header}");
        assert!(
            !header.contains("shipped_at"),
            "last column does not fit in 20 cols: {header}"
        );
    }

    #[test]
    fn scrolling_right_reveals_later_columns() {
        let lines = block_lines(&wide_result());
        let left = clip_table_block(&lines, &view(0, false, None), 24);
        let left_header = left
            .iter()
            .find(|l| l.contains("name"))
            .expect("header at offset 0");
        assert!(
            !left_header.contains("shipped_at"),
            "offset 0 must not show the last column: {left_header}"
        );

        // Push the offset far enough that the early columns leave the window.
        let right = clip_table_block(&lines, &view(14, false, None), 24);
        let right_header = right
            .iter()
            .find(|l| l.starts_with('│'))
            .expect("header at offset 14");
        assert!(
            right_header.contains("shipped_at"),
            "offset 14 must reveal the last column: {right_header}"
        );
        assert!(
            !right_header.contains("│ id"),
            "offset 14 must drop the first column: {right_header}"
        );
    }

    #[test]
    fn pinning_keeps_the_first_column_while_scrolling() {
        let lines = block_lines(&wide_result());
        let out = clip_table_block(&lines, &view(14, true, None), 28);
        let header = out
            .iter()
            .find(|l| l.starts_with('│'))
            .expect("header present");
        assert!(
            header.contains("│ id"),
            "pinned first column stays put: {header}"
        );
        assert!(
            header.contains("shipped_at"),
            "a far column is reached by scrolling: {header}"
        );
        assert!(
            !header.contains("name"),
            "a column between the pin and the scroll window is hidden: {header}"
        );
    }

    #[test]
    fn column_filter_keeps_only_named_columns() {
        let lines = block_lines(&wide_result());
        let out = clip_table_block(&lines, &view(0, false, Some(vec!["id", "total"])), 80);
        let header = out
            .iter()
            .find(|l| l.starts_with('│'))
            .expect("header present");
        assert!(header.contains("id"), "selected column id shows: {header}");
        assert!(
            header.contains("total"),
            "selected column total shows: {header}"
        );
        assert!(
            !header.contains("name"),
            "unselected column is hidden: {header}"
        );
        assert!(
            !header.contains("carrier"),
            "unselected column is hidden: {header}"
        );
    }

    #[test]
    fn a_column_filter_that_matches_nothing_falls_back_to_all() {
        let lines = block_lines(&wide_result());
        let out = clip_table_block(&lines, &view(0, false, Some(vec!["nope"])), 80);
        let header = out
            .iter()
            .find(|l| l.starts_with('│'))
            .expect("header present");
        assert!(
            header.contains("name"),
            "no match falls back to all columns: {header}"
        );
    }

    #[test]
    fn clipping_preserves_line_count() {
        let lines = block_lines(&wide_result());
        let wv = view(5, true, None);
        let out = clip_table_block(&lines, &wv, 30);
        assert_eq!(out.len(), lines.len(), "clipping never adds or drops lines");
    }

    #[test]
    fn a_non_box_block_is_hard_clipped_not_reconstructed() {
        let lines = vec!["(no columns) — 2 row(s)".to_string()];
        let out = clip_table_block(&lines, &view(0, false, None), 10);
        assert_eq!(out[0].chars().count(), 10);
        assert!(out[0].starts_with("(no column"));
    }
}
