//! One-line reconstruction for the visible columns: rows reuse plain `│`
//! edges, borders use open edges when the window is cut mid-table.

use super::geometry::clip_chars;

/// Rebuilds one box line for the visible columns. Rows use plain `│` edges;
/// borders use open edges (`├`/`┤`) when the window does not start at the first
/// or end at the last column, signalling that more columns lie off-screen.
pub(super) fn reconstruct_line(
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
