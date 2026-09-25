//! Header names of a box table block, in column order. Used by `/columns`
//! to report what is available.

use super::geometry::{cell_text, line_head, table_bounds};

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
