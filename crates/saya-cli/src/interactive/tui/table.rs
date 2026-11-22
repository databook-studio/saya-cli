mod box_render;
mod query;
mod shared;
#[cfg(test)]
mod tests;
mod wide;

pub(crate) use query::format_table;
pub(crate) use wide::{WideTableView, clip_table_block};

/// One shared row-count footer: `{n} row(s)`, with ` (truncated)` when the
/// rows shown are a capped prefix. Matches `query.rs`'s wording exactly —
/// a floor, never a total — so every surface reuses one form.
pub(crate) fn row_count_footer(row_count: usize, truncated: bool) -> String {
    if truncated {
        format!("{row_count} row(s) (truncated)")
    } else {
        format!("{row_count} row(s)")
    }
}
