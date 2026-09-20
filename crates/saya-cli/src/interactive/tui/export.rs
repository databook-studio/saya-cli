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

/// Renames repeated column labels so every key in one row is distinct:
/// the first `name` stays `name`; later repeats take `name_2`, `name_3`,
/// and so on. A repeated label must stay addressable after parsing —
/// duplicate object keys keep only the last value under ordinary JSON
/// parsing, silently dropping the earlier ones. All original labels are
/// reserved up front, so a generated suffix skips every name the query
/// itself used and an explicit `name_2` column is never shadowed.
fn disambiguated_columns(columns: &[String]) -> Vec<String> {
    let reserved: std::collections::HashSet<&str> = columns.iter().map(String::as_str).collect();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    columns
        .iter()
        .map(|column| {
            let next = counts.get(column.as_str()).copied().unwrap_or(0) + 1;
            counts.insert(column.as_str(), next);
            let mut candidate;
            if next == 1 {
                candidate = column.clone();
            } else {
                candidate = format!("{column}_{next}");
            }
            while seen.contains(&candidate)
                || (candidate != *column && reserved.contains(candidate.as_str()))
            {
                let bumped = counts.get(column.as_str()).copied().unwrap_or(1) + 1;
                counts.insert(column.as_str(), bumped);
                candidate = format!("{column}_{bumped}");
            }
            seen.insert(candidate.clone());
            candidate
        })
        .collect()
}

impl Serialize for JsonRow<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let names = disambiguated_columns(self.columns);
        let mut object = serializer.serialize_map(Some(names.len()))?;
        for (column, cell) in names.iter().zip(self.cells) {
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
    fn json_export_keeps_both_values_when_column_labels_repeat() {
        let result = QueryResult {
            columns: vec!["name".to_string(), "name".to_string()],
            rows: vec![json!(["first", "second"])],
            row_count: 1,
            truncated: false,
            executed_sql: "SELECT first AS name, second AS name".to_string(),
        };
        let path = std::env::temp_dir().join("saya_test_export_duplicate_values.json");

        write_result(&result, &path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        // The failure mode is a parsed document that lost a value while the
        // raw text looked right: assert through a real parser that both
        // values survive, addressable under distinct keys.
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        let row = &parsed[0];
        assert_eq!(row["name"], "first", "first value lost: {content}");
        assert_eq!(
            row["name_2"], "second",
            "second value lost or misnamed: {content}"
        );
    }

    #[test]
    fn json_export_never_shadows_a_column_the_query_itself_named() {
        let result = QueryResult {
            columns: vec!["name".to_string(), "name_2".to_string(), "name".to_string()],
            rows: vec![json!(["first", "explicit", "second"])],
            row_count: 1,
            truncated: false,
            executed_sql: "SELECT a, b, c".to_string(),
        };
        let path = std::env::temp_dir().join("saya_test_export_duplicate_shadow.json");

        write_result(&result, &path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        let row = &parsed[0];
        assert_eq!(row["name"], "first", "{content}");
        assert_eq!(row["name_2"], "explicit", "{content}");
        assert_eq!(row["name_3"], "second", "{content}");
    }

    #[test]
    fn json_export_keeps_the_explicit_label_when_duplicates_come_first() {
        // Q3: the reverse order of the shadowing test above. The explicit
        // `name_2` the query itself named must stay addressable with its own
        // value; the duplicate `name` takes the free suffix instead.
        let result = QueryResult {
            columns: vec!["name".to_string(), "name".to_string(), "name_2".to_string()],
            rows: vec![json!(["first", "second", "explicit"])],
            row_count: 1,
            truncated: false,
            executed_sql: "SELECT a, b, c".to_string(),
        };
        let path = std::env::temp_dir().join("saya_test_export_duplicate_leading.json");

        write_result(&result, &path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        let row = &parsed[0];
        assert_eq!(row["name"], "first", "{content}");
        assert_eq!(row["name_3"], "second", "{content}");
        assert_eq!(row["name_2"], "explicit", "{content}");
    }

    #[test]
    fn unique_column_labels_are_untouched_by_disambiguation() {
        assert_eq!(
            disambiguated_columns(&["id".to_string(), "name".to_string()]),
            vec!["id".to_string(), "name".to_string()]
        );
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
