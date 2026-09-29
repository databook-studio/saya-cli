//! Tests for the BigQuery request bodies: the parameter-free byte caps, the
//! positional `queryParameters` both the query and the dry-run must carry, and
//! the type mapping — including the refusals that never echo a value.

use saya_types::ConnectionError;
use serde_json::Value;

use crate::binds::{BindValue, parse_bind_values};
use saya_types::ParamValue;

use super::{dry_run_body, parse_result, query_body, query_parameters};

#[test]
fn query_body_carries_byte_cap_and_row_cap_as_strings() {
    let body = query_body("SELECT 1", 10, 1024, None, None);
    assert_eq!(body["query"], "SELECT 1");
    assert_eq!(body["useLegacySql"], false);
    // maxResults is one above the row cap so truncation is detectable.
    assert_eq!(body["maxResults"], 11);
    // BigQuery's int64 fields are formatted as strings.
    assert_eq!(body["maximumBytesBilled"], "1024");
    assert!(body.get("location").is_none());
    // A parameter-free body carries no mode and no list.
    assert!(body.get("parameterMode").is_none());
    assert!(body.get("queryParameters").is_none());
}

#[test]
fn query_body_includes_location_when_set() {
    let body = query_body("SELECT 1", 5, 1024, Some("EU"), None);
    assert_eq!(body["location"], "EU");
}

#[test]
fn dry_run_body_marks_dry_run_and_carries_byte_cap() {
    let body = dry_run_body("SELECT 1", 2048, None, None);
    assert_eq!(body["configuration"]["dryRun"], true);
    assert_eq!(body["configuration"]["query"]["query"], "SELECT 1");
    assert_eq!(body["configuration"]["query"]["maximumBytesBilled"], "2048");
}

#[test]
fn query_body_carries_positional_parameters() {
    let params = query_parameters(&[BindValue::Int(3), BindValue::Str("a".to_owned())]).unwrap();
    let body = query_body(
        "SELECT 1 WHERE a > ? AND b = ?",
        10,
        1024,
        None,
        Some(&params),
    );
    assert_eq!(body["parameterMode"], "POSITIONAL");
    assert_eq!(body["query"], "SELECT 1 WHERE a > ? AND b = ?");
    assert_eq!(
        body["queryParameters"],
        serde_json::json!([
            {"parameterType": {"type": "INT64"}, "parameterValue": {"value": "3"}},
            {"parameterType": {"type": "STRING"}, "parameterValue": {"value": "a"}},
        ])
    );
}

#[test]
fn dry_run_body_carries_the_identical_parameters() {
    let params = query_parameters(&[BindValue::Bool(true)]).unwrap();
    let body = dry_run_body("SELECT 1 WHERE a = ?", 2048, Some("US"), Some(&params));
    let query = body["configuration"]["query"].clone();
    assert_eq!(query["parameterMode"], "POSITIONAL");
    assert_eq!(query["location"], "US");
    // The dry run must see exactly the parameters the query will send, or its
    // byte estimate diverges from the real execution.
    let dry_params = query_parameters(&[BindValue::Bool(true)]).unwrap();
    let executed = query_body(
        "SELECT 1 WHERE a = ?",
        10,
        2048,
        Some("US"),
        Some(&dry_params),
    );
    assert_eq!(query["queryParameters"], executed["queryParameters"]);
    assert_eq!(query["parameterMode"], executed["parameterMode"]);
}

fn bound(value: ParamValue) -> BindValue {
    parse_bind_values(std::slice::from_ref(&value))
        .unwrap()
        .pop()
        .unwrap()
}

#[test]
fn parameters_map_to_the_bigquery_scalar_types() {
    let params = query_parameters(&[
        bound(ParamValue::Integer(12)),
        bound(ParamValue::Integer(-5)),
        bound(ParamValue::String("paris".to_owned())),
        bound(ParamValue::Boolean(true)),
        bound(ParamValue::Boolean(false)),
        bound(ParamValue::Decimal("007".to_owned())),
        bound(ParamValue::Decimal("1.5".to_owned())),
        bound(ParamValue::Date("2024-02-29".to_owned())),
        bound(ParamValue::Timestamp(
            "2024-01-01T02:00:00+02:00".to_owned(),
        )),
    ])
    .unwrap();
    let kind = |index: usize| params[index]["parameterType"]["type"].as_str().unwrap();
    let value = |index: usize| params[index]["parameterValue"]["value"].as_str().unwrap();
    assert_eq!((kind(0), value(0)), ("INT64", "12"));
    assert_eq!((kind(1), value(1)), ("INT64", "-5"));
    assert_eq!((kind(2), value(2)), ("STRING", "paris"));
    assert_eq!((kind(3), value(3)), ("BOOL", "true"));
    assert_eq!((kind(4), value(4)), ("BOOL", "false"));
    // A decimal binds its value as given; "007" is NUMERIC-legal.
    assert_eq!((kind(5), value(5)), ("NUMERIC", "007"));
    assert_eq!((kind(6), value(6)), ("NUMERIC", "1.5"));
    // A date binds its canonical YYYY-MM-DD form.
    assert_eq!((kind(7), value(7)), ("DATE", "2024-02-29"));
    // A timestamp binds the instant's canonical UTC RFC 3339 form.
    assert_eq!((kind(8), value(8)), ("TIMESTAMP", "2024-01-01T00:00:00Z"));
}

#[test]
fn wide_or_deep_decimals_bind_as_bignumeric() {
    let params = query_parameters(&[
        bound(ParamValue::Decimal(
            "1234567890123456789012345678901234567890".to_owned(),
        )),
        bound(ParamValue::Decimal("1.1234567890".to_owned())),
    ])
    .unwrap();
    assert_eq!(params[0]["parameterType"]["type"], "BIGNUMERIC");
    assert_eq!(params[1]["parameterType"]["type"], "BIGNUMERIC");
    let params = query_parameters(&[bound(ParamValue::Decimal("1e40".to_owned()))]).unwrap();
    assert_eq!(params[0]["parameterType"]["type"], "BIGNUMERIC");
}

#[test]
fn a_decimal_beyond_bignumeric_refuses_without_echoing_the_value() {
    let error = query_parameters(&[bound(ParamValue::Decimal("1e80".to_owned()))]).unwrap_err();
    assert!(
        matches!(error, ConnectionError::QueryFailed(_)),
        "{error:?}"
    );
    assert!(!error.to_string().contains("1e80"), "{error}");
    assert!(
        error.to_string().contains("BigQuery"),
        "the refusal must name the engine: {error}"
    );
}

#[test]
fn a_null_parameter_is_refused_on_bigquery() {
    // BigQuery requires a concrete parameterType for every parameter, and the
    // runtime value carries no declared type — Google's own client refuses an
    // untyped null, so the connector refuses before one round trip instead of
    // sending a guessed STRING-typed null that type-mismatches at compile time.
    let error = query_parameters(&[bound(ParamValue::Null)]).unwrap_err();
    assert!(
        matches!(error, ConnectionError::QueryFailed(_)),
        "{error:?}"
    );
    assert!(error.to_string().contains("BigQuery"), "{error}");
}

#[test]
fn parse_result_reads_columns_and_rows() {
    let body = serde_json::json!({
        "schema": {"fields": [{"name": "id", "type": "INTEGER"}, {"name": "name", "type": "STRING"}]},
        "rows": [{"f": [{"v": "1"}, {"v": "a"}]}, {"f": [{"v": "2"}, {"v": "b"}]}]
    });
    let result = parse_result(body, 10, "SELECT id, name FROM t".into());
    assert_eq!(result.columns, vec!["id", "name"]);
    assert_eq!(result.row_count, 2);
    assert!(!result.truncated);
    assert_eq!(result.rows[0], serde_json::json!(["1", "a"]));
    assert_eq!(result.rows[1], serde_json::json!(["2", "b"]));
}

#[test]
fn parse_result_keeps_columns_when_there_are_no_rows() {
    let body = serde_json::json!({"schema": {"fields": [{"name": "id"}]}, "rows": []});
    let result = parse_result(body, 10, "SELECT id FROM empty".into());
    assert_eq!(result.columns, vec!["id"]);
    assert_eq!(result.row_count, 0);
    assert!(!result.truncated);
}

#[test]
fn parse_result_caps_rows_and_marks_truncated() {
    let rows: Vec<Value> = (0..5)
        .map(|i| serde_json::json!({"f": [{"v": i.to_string()}]}))
        .collect();
    let body = serde_json::json!({"schema": {"fields": [{"name": "id"}]}, "rows": rows});
    let result = parse_result(body, 3, "SELECT id FROM t".into());
    assert_eq!(result.row_count, 3);
    assert!(result.truncated);
}

#[test]
fn parse_result_marks_truncated_on_page_token() {
    let body = serde_json::json!({
        "schema": {"fields": [{"name": "id"}]},
        "rows": [{"f": [{"v": "1"}]}],
        "pageToken": "next"
    });
    let result = parse_result(body, 10, "SELECT id FROM t".into());
    assert_eq!(result.row_count, 1);
    assert!(result.truncated);
}

#[test]
fn parse_result_missing_cell_becomes_null() {
    let body = serde_json::json!({
        "schema": {"fields": [{"name": "a"}, {"name": "b"}]},
        "rows": [{"f": [{"v": "1"}]}]
    });
    let result = parse_result(body, 10, "SELECT a, b FROM t".into());
    assert_eq!(result.rows[0], serde_json::json!(["1", null]));
}

#[test]
fn parse_result_marks_truncated_on_byte_budget() {
    let cell = "x".repeat(512 * 1024);
    let rows: Vec<Value> = (0..40)
        .map(|_| serde_json::json!({"f": [{"v": cell.clone()}]}))
        .collect();
    let body = serde_json::json!({"schema": {"fields": [{"name": "payload"}]}, "rows": rows});
    let result = parse_result(body, 100, "SELECT payload FROM t".into());
    assert!(result.truncated);
    assert!(result.row_count < 40);
}
