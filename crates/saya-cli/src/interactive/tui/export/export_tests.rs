//! Export tests, moved byte-identical: CSV/JSON round-trips, duplicate
//! label disambiguation, formula neutralisation, and the format gate.

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
        executed_sql: "SELECT value AS \"=HYPERLINK(\\\"http://x\\\",\\\"click\\\")\"".to_string(),
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
