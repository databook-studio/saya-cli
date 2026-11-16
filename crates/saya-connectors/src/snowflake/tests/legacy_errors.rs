use super::*;

#[tokio::test]
async fn legacy_login_and_query_failures_are_generic_and_secret_free() {
    let marker = "server-payload-sentinel";
    for login in [
        Reply::status("500 Internal Server Error", json!({"message":marker})),
        Reply::json(json!({"success":false,"message":marker})),
    ] {
        let (origin, _) = server(vec![login]).await;
        let mut item = connector(userpass());
        item.origin = origin;
        let error = item.connect().await.unwrap_err().to_string();
        assert!(!error.contains(marker));
        assert!(!error.contains("password-sentinel"));
    }
    for query in [
        Reply::status("500 Internal Server Error", json!({"message":marker})),
        Reply::json(json!({"success":false,"data":{"message":marker}})),
    ] {
        let login = Reply::json(json!({"success":true,"data":{"token":"session"}}));
        let (origin, _) = server(vec![login, query]).await;
        let mut item = connector(userpass());
        item.origin = origin;
        let error = item
            .execute(saya_types::QueryRequest::new("SELECT 1", 1))
            .await
            .unwrap_err()
            .to_string();
        assert!(!error.contains(marker));
        assert!(!error.contains("password-sentinel"));
    }
}

#[tokio::test]
async fn legacy_sql_compilation_error_names_the_object() {
    let login = Reply::json(json!({"success":true,"data":{"token":"session"}}));
    let query = Reply::json(json!({
        "success": false,
        "data": {
            "code": "001003",
            "message": "SQL compilation error: syntax error line 1 at position 0 unexpected 'SELCT'."
        }
    }));
    let (origin, _) = server(vec![login, query]).await;
    let mut item = connector(userpass());
    item.origin = origin;
    let error = item
        .execute(saya_types::QueryRequest::new("SELECT 1", 1))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("syntax error"), "opaque: {error}");
    assert!(error.contains("SELCT"), "opaque: {error}");
}

#[tokio::test]
async fn legacy_conversion_error_does_not_leak_row_value() {
    let login = Reply::json(json!({"success":true,"data":{"token":"session"}}));
    let query = Reply::json(json!({
        "success": false,
        "data": {
            "code": "100038",
            "message": "Numeric value '4111-1111-1111-1111' is not recognized"
        }
    }));
    let (origin, _) = server(vec![login, query]).await;
    let mut item = connector(userpass());
    item.origin = origin;
    let error = item
        .execute(saya_types::QueryRequest::new("SELECT 1", 1))
        .await
        .unwrap_err()
        .to_string();
    assert!(!error.contains("4111"), "row value leaked: {error}");
    assert_eq!(error, "query failed: Snowflake query failed");
}
