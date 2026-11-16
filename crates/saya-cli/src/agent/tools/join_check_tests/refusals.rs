//! The refusal seams: with data sharing off, join_check is refused; with
//! sharing on, a write statement is refused exactly as `bounded_sql_query`
//! refuses it — through the read-only safety layer.

use super::*;

use saya_agent::ToolExecutor;

#[tokio::test]
async fn join_check_is_refused_when_data_sharing_is_off() {
    let tools = DatabaseTools::new(None, 10, false);
    let error = tools
        .execute("join_check", serde_json::json!({"sql": "SELECT 1"}))
        .await
        .expect_err("join_check must be refused when data sharing is off");
    assert!(
        error.to_string().contains("data sharing is disabled"),
        "got: {error}"
    );
}

#[tokio::test]
async fn join_check_refuses_a_write_exactly_as_bounded_sql_query_does() {
    let tools = DatabaseTools::new(Some(Box::new(SafetyConnector)), 100, true);
    let write_sql = serde_json::json!({"sql": "DROP TABLE customers"});

    let join_err = tools
        .execute("join_check", write_sql.clone())
        .await
        .expect_err("join_check must refuse a write statement");
    let query_err = tools
        .execute("bounded_sql_query", write_sql)
        .await
        .expect_err("bounded_sql_query refuses the same write statement");

    assert_eq!(
        join_err, query_err,
        "join_check must refuse a write identically to bounded_sql_query"
    );
    assert!(
        join_err.to_string().contains("read-only safety policy"),
        "the refusal must come from the read-only safety layer: {join_err}"
    );
}
