use super::*;

#[tokio::test]
async fn result_shape_reports_the_connector_row_count_and_column_types() {
    let tools = DatabaseTools::new(Some(Box::new(TypedConnector)), 100, true);
    let shape = tools
        .execute(
            "result_shape",
            serde_json::json!({"sql": "SELECT customer, age, balance, active FROM customers"}),
        )
        .await
        .expect("result_shape should succeed");

    assert_eq!(shape["row_count"], 1);
    assert_eq!(shape["truncated"], false);
    let columns = shape["columns"]
        .as_array()
        .expect("columns is an array of name/type objects");
    assert_eq!(columns.len(), 4);
    assert_eq!(columns[0]["name"], "customer");
    assert_eq!(columns[0]["type"], "TEXT");
    assert_eq!(columns[1]["name"], "age");
    assert_eq!(columns[1]["type"], "INTEGER");
    assert_eq!(columns[2]["name"], "balance");
    assert_eq!(columns[2]["type"], "REAL");
    assert_eq!(columns[3]["name"], "active");
    assert_eq!(columns[3]["type"], "BOOLEAN");
}

#[tokio::test]
async fn result_shape_never_leaks_a_planted_cell_value() {
    let tools = DatabaseTools::new(Some(Box::new(SentinelConnector)), 100, true);
    let shape = tools
        .execute(
            "result_shape",
            serde_json::json!({"sql": "SELECT data FROM secrets"}),
        )
        .await
        .expect("result_shape should succeed");

    let serialized = serde_json::to_string(&shape).expect("shape serializes");
    assert!(
        !serialized.contains("SHAPE_LEAK_SENTINEL_42"),
        "a cell value escaped into the shape: {serialized}"
    );
    // The column name and an inferred type label are still present — the shape
    // is useful, only the values are suppressed.
    assert!(serialized.contains("data"));
    assert!(serialized.contains("TEXT"));
}

#[tokio::test]
async fn result_shape_reports_zero_rows_with_columns_still_present() {
    let tools = DatabaseTools::new(Some(Box::new(EmptyColumnsConnector)), 100, true);
    let shape = tools
        .execute(
            "result_shape",
            serde_json::json!({"sql": "SELECT * FROM empty"}),
        )
        .await
        .expect("result_shape should succeed on an empty result");

    assert_eq!(shape["row_count"], 0);
    assert_eq!(shape["truncated"], false);
    let columns = shape["columns"]
        .as_array()
        .expect("columns still present for an empty result");
    assert_eq!(columns.len(), 2);
    assert_eq!(columns[0]["name"], "customer");
    assert_eq!(columns[1]["name"], "age");
    // No row to infer a type from: the type label is null, never a value.
    assert_eq!(columns[0]["type"], serde_json::Value::Null);
    assert_eq!(columns[1]["type"], serde_json::Value::Null);
}

#[tokio::test]
async fn result_shape_sets_truncated_when_the_row_cap_is_hit() {
    let tools = DatabaseTools::new(Some(Box::new(CappingConnector { total: 5 })), 3, true);
    let shape = tools
        .execute(
            "result_shape",
            serde_json::json!({"sql": "SELECT id FROM big"}),
        )
        .await
        .expect("result_shape should succeed");

    assert_eq!(shape["row_count"], 3, "row_count is a floor at the cap");
    assert_eq!(
        shape["truncated"], true,
        "hitting the row cap sets truncated"
    );
}
