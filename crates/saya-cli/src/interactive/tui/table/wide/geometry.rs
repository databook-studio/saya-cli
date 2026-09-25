//! Box-grid primitives: boundary detection, boundary columns, cell text,
//! and hard character clipping. Shared by the clip, column, render, and
//! names concerns.

/// Box-drawing chars that sit on a column boundary. They occur at the same
/// character columns in every line of one table, so any border or row line
/// yields the same boundary set.
fn is_boundary(ch: char) -> bool {
    matches!(
        ch,
        '│' | '┌' | '┐' | '└' | '┘' | '├' | '┤' | '┬' | '┴' | '┼'
    )
}

pub(super) fn line_head(line: &str) -> char {
    line.chars().next().unwrap_or(' ')
}

/// Boundary character columns, taken from the separator line when present (it
/// never carries cell content) and falling back to the header line.
pub(super) fn table_bounds(box_lines: &[String]) -> Vec<usize> {
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

pub(super) fn cell_text(chars: &[char], bounds: &[usize], col: usize) -> String {
    let start = bounds[col] + 1;
    let end = bounds[col + 1];
    let s: String = chars[start..end].iter().collect();
    s.trim().to_string()
}

pub(super) fn clip_chars(s: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    s.chars().take(width).collect()
}
