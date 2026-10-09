use super::*;

/// The row cap is appended after the placeholders and never shifts the bind
/// order: values land on their markers positionally.
#[tokio::test]
async fn bounds_rewrite_preserves_bind_order() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("params_order.db");
    create_param_fixture(&path).await;
    let connector = open_param_connector(&path).await;

    let request = QueryRequest::with_params(
        "SELECT :a AS a, :b AS b".to_string(),
        10,
        vec![
            bound("a", ParamValue::Integer(1)),
            bound("b", ParamValue::Integer(2)),
        ],
    );
    let result = connector.execute(request).await.unwrap();
    assert_eq!(only_row(&result), &vec![Value::from(1), Value::from(2)]);

    // Two rows exist, `max_rows` is 1, so the cap appends `LIMIT 2` after the
    // placeholder. If the appended bound disturbed the order, `id` would not
    // see 2 — and the cap must still truncate the two matching rows.
    let request = QueryRequest::with_params(
        "SELECT id FROM params_t WHERE id <= :id ORDER BY id".to_string(),
        1,
        vec![bound("id", ParamValue::Integer(2))],
    );
    let result = connector.execute(request).await.unwrap();
    assert_eq!(only_row(&result)[0], Value::from(1));
    assert!(result.truncated, "the row cap must still apply");

    drop(connector);
    drop(temp_dir);
}

/// Engines whose native binding has not landed refuse parameters with an
/// unsupported error before any connection attempt — a ClickHouse profile
/// pointed at a port with nothing listening must fail as unsupported, not as
/// a network error. DuckDB binds natively now, so it no longer appears here.
#[tokio::test]
async fn parameters_on_an_unsupported_connector_are_refused_without_connecting() {
    let clickhouse = build_connector(
        &DatabaseProfile::ClickHouse {
            host: "127.0.0.1".into(),
            port: Some(1),
            database: None,
            user: None,
            password: None,
            secure: Some(false),
        },
        &MapSecretResolver::new([]),
        ConnectorOptions::default(),
    )
    .await
    .unwrap();
    assert!(!clickhouse.supports_parameters());
    let error = clickhouse
        .execute(QueryRequest::with_params(
            "SELECT 1 WHERE 1 = :v".to_string(),
            1,
            vec![bound("v", ParamValue::Integer(1))],
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(error, ConnectionError::Unsupported(_)),
        "{error:?}"
    );
    assert!(
        error
            .to_string()
            .contains("parameters are not supported for ClickHouse"),
        "{error}"
    );
}

/// The SQLite connector is one of the engines wired for native binding.
#[tokio::test]
async fn sqlite_supports_parameters() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("params_capability.db");
    create_param_fixture(&path).await;
    let connector = open_param_connector(&path).await;
    assert!(connector.supports_parameters());

    drop(connector);
    drop(temp_dir);
}
