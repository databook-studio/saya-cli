#[test]
fn tool_call_detail_surfaces_the_sql() {
    // Single-connection query: the SQL, whitespace collapsed to one line.
    let detail = tool_call_detail(
        "bounded_sql_query",
        &serde_json::json!({"sql": "SELECT *\n  FROM   users"}),
    )
    .expect("query tools expose their SQL");
    assert_eq!(detail, "SELECT * FROM users");

    // A named connection is annotated.
    let detail = tool_call_detail(
        "bounded_sql_query",
        &serde_json::json!({"sql": "SELECT 1", "connection": "warehouse"}),
    )
    .unwrap();
    assert!(
        detail.contains("SELECT 1") && detail.contains("@warehouse"),
        "got: {detail}"
    );

    // The fan-out tool labels itself as running everywhere.
    let detail = tool_call_detail(
        "bounded_sql_query_all",
        &serde_json::json!({"sql": "SELECT 1"}),
    )
    .unwrap();
    assert!(detail.contains("all connected databases"), "got: {detail}");

    // result_shape is a SQL tool like bounded_sql_query, so its SQL surfaces too.
    let detail = tool_call_detail(
        "result_shape",
        &serde_json::json!({"sql": "SELECT 1", "connection": "warehouse"}),
    )
    .unwrap();
    assert!(
        detail.contains("SELECT 1") && detail.contains("@warehouse"),
        "got: {detail}"
    );

    // Tools without a query expose no detail.
    assert!(tool_call_detail("schema_discovery", &serde_json::json!({})).is_none());
}

#[tokio::test]
async fn test_database_tools_multi_connection_routing() {
    let mut registry = ConnectionRegistry::new("primary");
    registry.insert(
        "primary",
        ConnectionEntry {
            connector: Box::new(FakeConnector {
                table_name: "from_primary".into(),
            }),
            dialect: SqlDialect::DuckDb,
            profile_id: None,
        },
    );
    registry.insert(
        "warehouse",
        ConnectionEntry {
            connector: Box::new(FakeConnector {
                table_name: "from_secondary".into(),
            }),
            dialect: SqlDialect::Postgres,
            profile_id: None,
        },
    );

    let tools = DatabaseTools::with_registry(registry, 100, true, None);

    let res = tools
        .execute("schema_discovery", serde_json::json!({}))
        .await
        .expect("schema discovery on primary should succeed");
    assert!(
        res.to_string().contains("from_primary"),
        "expected result to contain 'from_primary', got: {res}"
    );

    let res = tools
        .execute(
            "schema_discovery",
            serde_json::json!({"connection": "warehouse"}),
        )
        .await
        .expect("schema discovery on warehouse should succeed");
    assert!(
        res.to_string().contains("from_secondary"),
        "expected result to contain 'from_secondary', got: {res}"
    );

    let err = tools
        .execute(
            "schema_discovery",
            serde_json::json!({"connection": "nope"}),
        )
        .await
        .expect_err("unknown connection should return error");
    let err_str = err.to_string();
    assert!(
        err_str.contains("primary") && err_str.contains("warehouse"),
        "error message should list available connections, got: {err_str}"
    );
}
