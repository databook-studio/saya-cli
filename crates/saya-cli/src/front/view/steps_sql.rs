//! A query's SQL as one line: whitespace collapsed, cut to the width the line
//! has, and marked with a single ellipsis when the cut happened.

use crate::agent::tools::collapse_whitespace;

/// `sql` on one line of at most `width` columns, with one `…` where it was cut.
///
/// The collapse is the SQL reader's own ([`collapse_whitespace`]), so a quoted
/// value keeps the spaces it was written with; only the newline that ends a
/// line comment has to go, and one space stands in for it.
pub(crate) fn one_line(sql: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let flat = flatten(sql);
    if flat.chars().count() <= width {
        return flat;
    }
    let mut cut: String = flat.chars().take(width - 1).collect();
    cut.push('…');
    cut
}

/// `sql` with its whitespace collapsed and its lines joined by a space.
fn flatten(sql: &str) -> String {
    let collapsed = collapse_whitespace(sql);
    let mut out = String::with_capacity(collapsed.len());
    for ch in collapsed.trim().chars() {
        if ch == '\n' || ch == '\r' {
            if !out.is_empty() && !out.ends_with(' ') {
                out.push(' ');
            }
        } else {
            out.push(ch);
        }
    }
    out
}
