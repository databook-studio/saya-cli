//! Tests for the bounded query contract and its parameter bindings.

use crate::params::{BoundParam, ParamValue};

use super::{QueryRequest, QueryResult};

#[test]
fn new_has_no_params_and_serializes_without_the_field() {
    let request = QueryRequest::new("select 1", 10);
    assert!(request.params.is_empty());
    let json = serde_json::to_string(&request).expect("a request serializes");
    assert_eq!(json, r#"{"sql":"select 1","max_rows":10}"#);
    let back: QueryRequest = serde_json::from_str(&json).expect("the request reparses");
    assert_eq!(back, request);
}

#[test]
fn with_params_round_trips_through_json() {
    let request = QueryRequest::with_params(
        "select :region",
        10,
        vec![BoundParam {
            name: "region".to_owned(),
            value: ParamValue::String("eu".to_owned()),
        }],
    );
    let json = serde_json::to_string(&request).expect("a request serializes");
    assert!(
        json.contains("\"params\":["),
        "bound params serialize: {json}"
    );
    let back: QueryRequest = serde_json::from_str(&json).expect("the request reparses");
    assert_eq!(back, request);
}

#[test]
fn missing_params_field_deserializes_as_empty() {
    let bare: QueryRequest =
        serde_json::from_str(r#"{"sql":"select 1","max_rows":10}"#).expect("old shape parses");
    assert!(bare.params.is_empty());
    assert_eq!(bare.sql, "select 1");
    assert_eq!(bare.max_rows, 10);
}

#[test]
fn debug_redacts_bound_values() {
    let request = QueryRequest::with_params(
        "select :region",
        10,
        vec![BoundParam {
            name: "region".to_owned(),
            value: ParamValue::String("secret-value".to_owned()),
        }],
    );
    let rendered = format!("{request:?}");
    assert!(
        !rendered.contains("secret-value"),
        "bound values must not render: {rendered}"
    );
    assert!(
        rendered.contains("region"),
        "the parameter name may render: {rendered}"
    );
}

#[test]
fn query_result_is_untouched_by_parameters() {
    let result = QueryResult::empty("select :region");
    assert!(result.rows.is_empty());
    assert_eq!(result.executed_sql, "select :region");
}
