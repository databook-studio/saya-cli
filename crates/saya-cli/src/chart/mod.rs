//! Generates self-contained interactive Chart.js HTML documents.

mod cleanup;
mod kind;
mod render;
mod spec;
mod temp_chart;
#[cfg(test)]
pub(crate) mod test_support;

pub(crate) use cleanup::cleanup_session_charts;
pub(crate) use kind::{ChartKind, ChartSpec};
pub(crate) use render::render_html;
pub(crate) use spec::suggest_spec;
pub(crate) use temp_chart::reserve_temp_chart;

pub(super) fn is_numeric_column(rows: &[Vec<serde_json::Value>], col_idx: usize) -> bool {
    let mut non_null_count = 0;
    for row in rows {
        if let Some(cell) = row.get(col_idx).filter(|c| !c.is_null()) {
            non_null_count += 1;
            if cell_to_f64(cell).is_none() {
                return false;
            }
        }
    }
    non_null_count >= 1
}

/// Interprets a cell as an `f64`, accepting both JSON numbers and numeric
/// *strings*. Postgres `NUMERIC`/`DECIMAL` and MySQL `DECIMAL` (e.g. `SUM`/`AVG`,
/// money columns) decode to JSON strings, so a number-only check would silently
/// drop them from charts.
pub(super) fn cell_to_f64(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(_) => value.as_f64(),
        serde_json::Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

pub(super) fn normalize_row(row: &serde_json::Value, col_count: usize) -> Vec<serde_json::Value> {
    let mut cells = match row {
        serde_json::Value::Array(arr) => arr.clone(),
        scalar => vec![scalar.clone()],
    };
    cells.resize(col_count, serde_json::Value::Null);
    cells
}

/// Writes the HTML document string to the specified path.
#[allow(dead_code)]
pub(crate) fn write_html(html: &str, path: &std::path::Path) -> Result<(), String> {
    std::fs::write(path, html).map_err(|e| format!("failed to write chart file: {e}"))
}

/// Opens `path` in the user's default application (browser for .html).
pub(crate) fn open_file(path: &std::path::Path) -> Result<(), String> {
    use std::process::Command;
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = Command::new("open");
        c.arg(path);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut c = Command::new("xdg-open");
        c.arg(path);
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", ""]).arg(path);
        c
    };
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("could not open {}: {e}", path.display()))
}

#[cfg(test)]
mod tests;
