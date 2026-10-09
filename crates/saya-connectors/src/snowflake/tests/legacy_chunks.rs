use super::*;

#[tokio::test]
async fn legacy_relogs_once_decodes_gzip_chunks_and_forwards_only_ssec_headers() {
    let login = || Reply::json(json!({"success":true,"data":{"token":"session"}}));
    let expired = Reply::json(json!({"success":false,"data":{"code":"390104"}}));
    let (origin, queue, seen) = dynamic_server().await;
    // Chunk URLs are same-origin, as they are in production deployments.
    queue.lock().await.push(login());
    queue.lock().await.push(expired);
    queue.lock().await.push(login());
    queue.lock().await.push(Reply::json(
        json!({"success":true,"data":{"rowtype":[{"name":"N"}],"rowset":[[1]],"chunkHeaders":{"x-amz-server-side-encryption-customer-key":"key","x-amz-server-side-encryption-customer-key-md5":"md5","x-not-forwarded":"nope"},"chunks":[{"url":format!("{origin}/chunks/1")},{"url":format!("{origin}/chunks/2")}]}}),
    ));
    queue.lock().await.push(Reply::gzip("[2],"));
    queue.lock().await.push(Reply::gzip("[3],"));
    let mut item = connector(userpass());
    item.origin = origin.clone();
    item.context = Context {
        warehouse: Some("WH".into()),
        database: Some("DB".into()),
        schema: Some("SCH".into()),
        role: Some("ROLE".into()),
    };
    let output = item
        .execute(saya_types::QueryRequest::new("SELECT 1", 3))
        .await
        .unwrap();
    assert_eq!(output.columns, vec!["N"]);
    assert_eq!(output.rows, rows(&[1, 2, 3]));
    let requests = seen.lock().await;
    assert_eq!(
        requests
            .iter()
            .filter(|item| item.contains("/session/v1/login-request"))
            .count(),
        2
    );
    assert!(
        requests
            .iter()
            .filter(|item| item.contains("/queries/v1/query-request"))
            .all(|item| item.contains("LIMIT 4"))
    );
    let first = requests
        .iter()
        .find(|item| item.contains("/chunks/1"))
        .expect("first chunk fetched")
        .to_ascii_lowercase();
    assert!(first.contains("x-amz-server-side-encryption-customer-key: key"));
    assert!(first.contains("x-amz-server-side-encryption-customer-key-md5: md5"));
    assert!(!first.contains("x-not-forwarded"));
    assert_eq!(
        requests
            .iter()
            .filter(|item| item.contains("/chunks/2"))
            .count(),
        1
    );
    let login_request = &requests[0];
    let login_json: Value = serde_json::from_str(request_body(login_request)).unwrap();
    assert_eq!(login_json["data"]["WAREHOUSE_NAME"], "WH");
    assert!(login_json["data"].get("WAREHOUSE").is_none());
    assert!(login_request.contains("warehouse=WH"));
    assert!(login_request.contains("databaseName=DB"));
    assert!(login_request.contains("schemaName=SCH"));
    assert!(login_request.contains("roleName=ROLE"));
}

#[tokio::test]
async fn legacy_401_relogs_once_early_stops_and_redacts_chunk_failures() {
    let login = || Reply::json(json!({"success":true,"data":{"token":"session"}}));
    let response = |url: &str| {
        Reply::json(
            json!({"success":true,"data":{"rowtype":[{"name":"N"}],"rowset":[[1],[2]],"chunks":[{"url":url}]}}),
        )
    };
    let (origin, seen) = server(vec![
        login(),
        Reply::status("401 Unauthorized", json!({"marker":"secret"})),
        login(),
        response("http://127.0.0.1:9/unused"),
    ])
    .await;
    let mut item = connector(userpass());
    item.origin = origin;
    let output = item
        .execute(saya_types::QueryRequest::new("SELECT 1", 1))
        .await
        .unwrap();
    assert!(output.truncated);
    assert_eq!(
        seen.lock()
            .await
            .iter()
            .filter(|request| request.contains("login-request"))
            .count(),
        2
    );
    let (origin, queue, seen) = dynamic_server().await;
    queue.lock().await.push(login());
    queue.lock().await.push(Reply::json(
        json!({"success":true,"data":{"rowtype":[{"name":"N"}],"rowset":[[1],[2]],"chunks":[{"url":format!("{origin}/chunks/failing")}]}}),
    ));
    queue.lock().await.push(Reply::status(
        "500 Internal Server Error",
        json!({"marker":"chunk-secret"}),
    ));
    let mut item = connector(userpass());
    item.origin = origin.clone();
    let error = item
        .execute(saya_types::QueryRequest::new("SELECT 1", 3))
        .await
        .unwrap_err();
    assert!(!error.to_string().contains("chunk-secret"));
    // The chunk was genuinely fetched (and failed), not skipped by validation.
    assert_eq!(
        seen.lock()
            .await
            .iter()
            .filter(|request| request.contains("/chunks/failing"))
            .count(),
        1
    );
    let (origin, queue, seen) = dynamic_server().await;
    queue.lock().await.push(login());
    queue.lock().await.push(Reply::json(
        json!({"success":true,"data":{"rowtype":[{"name":"N"}],"rowset":[[1],[2]],"chunks":[{"url":format!("{origin}/chunks/garbage")}]}}),
    ));
    queue.lock().await.push(Reply::gzip("not-json-secret"));
    let mut item = connector(userpass());
    item.origin = origin;
    let error = item
        .execute(saya_types::QueryRequest::new("SELECT 1", 3))
        .await
        .unwrap_err();
    assert!(!error.to_string().contains("not-json-secret"));
    assert_eq!(
        seen.lock()
            .await
            .iter()
            .filter(|request| request.contains("/chunks/garbage"))
            .count(),
        1
    );
    assert!(item.active.lock().await.is_none());
}

#[tokio::test]
async fn schema_maps_catalog_rows_and_escapes_context_identifiers() {
    let response = Reply::json(v2_done(&[0], 0));
    let mut body: Value = serde_json::from_slice(&response.body).unwrap();
    body["data"] = json!([
        ["CAT", "SCH'EMA", "T", "C", "TEXT", "YES"],
        ["CAT", "SCH'EMA", "T", "D", "NUMBER", "NO"]
    ]);
    let (origin, seen) = server(vec![Reply::json(body)]).await;
    let mut item = connector(Auth::Keypair(keypair()));
    item.origin = origin;
    item.context.database = Some("CAT\"ALOG".into());
    item.context.schema = Some("SCH'EMA".into());
    let tree = item.schema().await.unwrap();
    assert_eq!(tree.databases[0].name, "CAT\"ALOG");
    assert_eq!(tree.databases[0].schemas[0].tables[0].columns.len(), 2);
    let request = seen.lock().await[0].clone();
    let posted: Value = serde_json::from_str(request_body(&request)).unwrap();
    let sql = posted["statement"].as_str().unwrap();
    assert!(sql.contains("\"CAT\"\"ALOG\".INFORMATION_SCHEMA.COLUMNS"));
    assert!(sql.contains("table_schema = 'SCH''EMA'"));
}
