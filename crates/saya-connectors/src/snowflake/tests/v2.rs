use super::*;

#[tokio::test]
async fn v2_protocol_polls_partitions_and_preserves_wire_contract() {
    let pending = Reply::status(
        "202 Accepted",
        json!({"statementHandle":handle(), "statementStatusUrl":"/api/v2/statements/status"}),
    );
    let done = Reply::json(v2_done(&[1], 2));
    let part = Reply::gzip(
        &json!({"data":[[2]], "resultSetMetaData":{"rowType":[{"name":"N"}]}}).to_string(),
    );
    let (origin, seen) = server(vec![pending, done, part]).await;
    let mut item = connector(Auth::Keypair(keypair()));
    item.origin = origin;
    let output = item
        .execute(saya_types::QueryRequest::new("SELECT 1", 2))
        .await
        .unwrap();
    assert_eq!(output.columns, vec!["N"]);
    assert_eq!(output.rows, rows(&[1, 2]));
    assert!(!output.truncated);
    let traffic = seen.lock().await;
    assert_eq!(traffic.len(), 3);
    assert!(traffic[0].starts_with("POST /api/v2/statements HTTP/1.1"));
    assert!(
        traffic[0].contains("user-agent: saya-cli/0.1")
            || traffic[0].contains("User-Agent: saya-cli/0.1")
    );
    assert!(
        traffic[0]
            .to_ascii_lowercase()
            .contains("accept: application/json")
    );
    assert!(
        traffic[0].contains("authorization: Bearer ")
            || traffic[0].contains("Authorization: Bearer ")
    );
    assert!(
        traffic[0]
            .to_ascii_lowercase()
            .contains("x-snowflake-authorization-token-type: keypair_jwt")
    );
    assert!(traffic[0].contains("LIMIT 3"));
    assert!(traffic[1].starts_with("GET /api/v2/statements/status HTTP/1.1"));
    assert!(traffic[2].starts_with(&format!(
        "GET /api/v2/statements/{}?partition=1 HTTP/1.1",
        handle()
    )));
    assert_eq!(request_body(&traffic[1]), "");
    assert_eq!(request_body(&traffic[2]), "");
}

#[tokio::test]
async fn v2_caps_rows_before_later_partitions() {
    let (origin, seen) = server(vec![Reply::json(v2_done(&[1, 2], 3))]).await;
    let mut item = connector(Auth::Keypair(keypair()));
    item.origin = origin;
    let output = item
        .execute(saya_types::QueryRequest::new("SELECT 1", 1))
        .await
        .unwrap();
    assert_eq!(output.rows, rows(&[1]));
    assert!(output.truncated);
    assert_eq!(seen.lock().await.len(), 1);
}

#[tokio::test]
async fn v2_rejects_cross_origin_and_prefix_trick_without_contacting_them() {
    for location in [
        "http://localhost:9/api/v2/statements/status",
        "http://127.0.0.1.evil/status",
    ] {
        let (origin, seen) = server(vec![Reply::status(
            "202 Accepted",
            json!({"statementHandle":handle(), "statementStatusUrl":location}),
        )])
        .await;
        let mut item = connector(Auth::Keypair(keypair()));
        item.origin = origin;
        let error = item
            .execute(saya_types::QueryRequest::new("SELECT 1", 1))
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "query failed: Snowflake query failed");
        assert_eq!(seen.lock().await.len(), 1);
    }
}

#[tokio::test]
async fn v2_errors_timeout_and_partition_failures_are_redacted_and_clear_active() {
    let marker = "LEAK-MARKER";
    for reply in [
        Reply::status("422 Unprocessable Content", json!({"message":marker})),
        Reply {
            status: "200 OK",
            body: marker.as_bytes().to_vec(),
            headers: vec![],
            delay: Duration::ZERO,
        },
    ] {
        let (origin, _) = server(vec![reply]).await;
        let mut item = connector(Auth::Keypair(keypair()));
        item.origin = origin;
        let error = item
            .execute(saya_types::QueryRequest::new("SELECT 1", 1))
            .await
            .unwrap_err();
        assert!(!error.to_string().contains(marker));
        assert!(item.active.lock().await.is_none());
    }
    let pending = Reply::status(
        "202 Accepted",
        json!({"statementHandle":handle(), "statementStatusUrl":"/status"}),
    );
    let (origin, _) = server(vec![
        pending,
        Reply::status(
            "202 Accepted",
            json!({"statementHandle":handle(), "statementStatusUrl":"/status"}),
        ),
    ])
    .await;
    let mut item = connector(Auth::Keypair(keypair()));
    item.origin = origin;
    item.timeout = Duration::from_millis(70);
    assert!(
        item.execute(saya_types::QueryRequest::new("SELECT 1", 1))
            .await
            .is_err()
    );
    assert!(item.active.lock().await.is_none());
    let (origin, _) = server(vec![
        Reply::json(v2_done(&[1], 2)),
        Reply::status("500 Internal Server Error", json!({"error":marker})),
    ])
    .await;
    let mut item = connector(Auth::Keypair(keypair()));
    item.origin = origin;
    let error = item
        .execute(saya_types::QueryRequest::new("SELECT 1", 2))
        .await
        .unwrap_err();
    assert!(!error.to_string().contains(marker));
    assert!(item.active.lock().await.is_none());
}

#[tokio::test]
async fn v2_sql_compilation_error_names_the_missing_object() {
    let (origin, _) = server(vec![Reply::status(
        "422 Unprocessable Content",
        json!({
            "code": "002003",
            "message": "SQL compilation error:\nObject 'ORDRS' does not exist or not authorized.",
            "success": false
        }),
    )])
    .await;
    let mut item = connector(Auth::Keypair(keypair()));
    item.origin = origin;
    let error = item
        .execute(saya_types::QueryRequest::new("SELECT * FROM ordrs", 1))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("ORDRS"), "object name dropped: {error}");
    assert!(error.contains("does not exist"), "opaque: {error}");
}

#[tokio::test]
async fn v2_conversion_error_does_not_leak_row_value() {
    let (origin, _) = server(vec![Reply::status(
        "422 Unprocessable Content",
        json!({
            "code": "100038",
            "message": "Numeric value '4111-1111-1111-1111' is not recognized",
            "success": false
        }),
    )])
    .await;
    let mut item = connector(Auth::Keypair(keypair()));
    item.origin = origin;
    let error = item
        .execute(saya_types::QueryRequest::new("SELECT 1", 1))
        .await
        .unwrap_err()
        .to_string();
    assert!(!error.contains("4111"), "row value leaked: {error}");
    assert_eq!(error, "query failed: Snowflake query failed");
}

#[tokio::test]
async fn cancellation_uses_uuid_endpoint_and_rejects_invalid_or_timed_out_handles() {
    let (origin, seen) = server(vec![Reply::json(json!({"ok":true}))]).await;
    let mut item = connector(Auth::Keypair(keypair()));
    item.origin = origin;
    *item.active.lock().await = Some(handle().into());
    assert_eq!(
        item.request_cancel().await.unwrap(),
        crate::CancelRequestOutcome::RemoteRequestAccepted
    );
    let request = seen.lock().await[0].clone();
    assert!(request.starts_with(&format!("POST /api/v2/statements/{}/cancel", handle())));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("x-snowflake-authorization-token-type: keypair_jwt")
    );
    *item.active.lock().await = Some("not-a-uuid".into());
    assert!(
        item.request_cancel()
            .await
            .unwrap_err()
            .to_string()
            .contains("no active")
    );
    let (origin, _) = server(vec![Reply {
        status: "200 OK",
        body: b"{}".to_vec(),
        headers: vec![],
        delay: Duration::from_millis(120),
    }])
    .await;
    let mut timed = connector(Auth::Keypair(keypair()));
    timed.origin = origin;
    timed.timeout = Duration::from_millis(30);
    *timed.active.lock().await = Some(handle().into());
    assert!(timed.request_cancel().await.is_err());
}
