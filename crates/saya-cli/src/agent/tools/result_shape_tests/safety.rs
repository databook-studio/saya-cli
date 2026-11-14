use super::*;

#[tokio::test]
async fn result_shape_refuses_a_write_exactly_as_bounded_sql_query_does() {
    let tools = DatabaseTools::new(Some(Box::new(SafetyConnector)), 100, true);
    let write_sql = serde_json::json!({"sql": "DROP TABLE customers"});

    let shape_err = tools
        .execute("result_shape", write_sql.clone())
        .await
        .expect_err("result_shape must refuse a write statement");
    let query_err = tools
        .execute("bounded_sql_query", write_sql)
        .await
        .expect_err("bounded_sql_query refuses the same write statement");

    assert_eq!(
        shape_err, query_err,
        "result_shape must refuse a write identically to bounded_sql_query"
    );
    assert!(
        shape_err.to_string().contains("read-only safety policy"),
        "the refusal must come from the read-only safety layer: {shape_err}"
    );
}

#[tokio::test]
async fn result_shape_is_refused_when_data_sharing_is_off() {
    let tools = DatabaseTools::new(None, 10, false);
    let error = tools
        .execute("result_shape", serde_json::json!({"sql": "SELECT 1"}))
        .await
        .expect_err("result_shape must be refused when data sharing is off");
    assert!(
        error.to_string().contains("data sharing is disabled"),
        "got: {error}"
    );
}
