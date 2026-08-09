//! Writes a query result to a file as CSV or JSON, chosen by extension.

use saya_types::QueryResult;
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
        .map(|c| escape_csv_field(c))
        .collect::<Vec<_>>()
        .join(",");
    let mut lines = vec![header];
    for row in &result.rows {
        let cells = normalize_row(row, col_count);
        let line = cells
            .iter()
            .map(|cell| escape_csv_field(&cell_to_csv_string(cell)))
            .collect::<Vec<_>>()
            .join(",");
        lines.push(line);
    }
    let content = lines.join("\n") + "\n";
    std::fs::write(path, content).map_err(|e| format!("failed to write CSV file: {e}"))?;
    Ok(result.rows.len())
}

fn export_json(result: &QueryResult, path: &Path) -> Result<usize, String> {
    let col_count = result.columns.len();
    let mut objects = Vec::with_capacity(result.rows.len());
    for row in &result.rows {
        let cells = normalize_row(row, col_count);
        let mut map = serde_json::Map::new();
        for (i, col) in result.columns.iter().enumerate() {
            map.insert(col.clone(), cells[i].clone());
        }
        objects.push(serde_json::Value::Object(map));
    }
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
}
