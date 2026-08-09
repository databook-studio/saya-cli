use super::box_render::render_box;
use super::shared::{cell_text, get_cell};
use saya_types::QueryResult;

/// Formats a query result as an aligned, box-drawn table (monospace text) for
/// the transcript. Columns are padded to their content width (capped), long
/// cells are truncated with an ellipsis, and a row-count footer is appended.
/// Numeric columns are right-aligned; text and header cells are left-aligned.
pub(crate) fn format_table(result: &QueryResult) -> String {
    let num_cols = result.columns.len();
    let row_count = result.rows.len();
    let trunc_suffix = if result.truncated { " (truncated)" } else { "" };

    if num_cols == 0 {
        return format!("(no columns) — {row_count} row(s){trunc_suffix}");
    }

    let numeric: Vec<bool> = (0..num_cols)
        .map(|col_idx| {
            let mut has_numeric = false;
            let mut all_non_null_numeric = true;

            for row_val in &result.rows {
                if let Some(cell) = get_cell(row_val, col_idx) {
                    match cell {
                        serde_json::Value::Null => {}
                        serde_json::Value::Number(_) => has_numeric = true,
                        _ => all_non_null_numeric = false,
                    }
                }
            }

            has_numeric && all_non_null_numeric
        })
        .collect();

    let rows: Vec<Vec<String>> = result
        .rows
        .iter()
        .map(|row_val| {
            let mut cells = match row_val {
                serde_json::Value::Array(vals) => vals.iter().map(cell_text).collect::<Vec<_>>(),
                other => vec![cell_text(other)],
            };
            if cells.len() < num_cols {
                cells.resize(num_cols, String::new());
            } else {
                cells.truncate(num_cols);
            }
            cells
        })
        .collect();

    let mut lines = render_box(&result.columns, &rows, &numeric);
    lines.push(format!("{row_count} row(s){trunc_suffix}"));
    lines.join("\n")
}
