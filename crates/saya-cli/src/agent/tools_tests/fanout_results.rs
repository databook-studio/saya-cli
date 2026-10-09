#[tokio::test]
async fn bounded_sql_query_all_fans_out_over_every_connection() {
    let tools = DatabaseTools::with_registry(two_connection_registry(), 100, true, None);

    let res = tools
        .execute(
            "bounded_sql_query_all",
            serde_json::json!({"sql": "SELECT 1"}),
        )
        .await
        .expect("fan-out query should succeed");

    let databases = res
        .get("databases")
        .and_then(|value| value.as_array())
        .expect("result should carry a `databases` array");
    assert_eq!(databases.len(), 2, "one entry per connected database");

    let names: Vec<&str> = databases
        .iter()
        .filter_map(|db| db.get("connection").and_then(|v| v.as_str()))
        .collect();
    assert_eq!(
        names,
        vec!["primary", "warehouse"],
        "insertion order preserved"
    );
    // Every database reports its dialect and a successful result, not an error.
    for db in databases {
        assert!(db.get("dialect").is_some(), "each entry names its dialect");
        assert!(db.get("result").is_some(), "each entry carries a result");
        assert!(db.get("error").is_none(), "no errors expected here");
    }
}

#[tokio::test]
async fn bounded_sql_query_all_reports_per_database_errors_without_aborting() {
    let mut registry = ConnectionRegistry::new("primary");
    registry.insert(
        "primary",
        ConnectionEntry {
            connector: Box::new(FakeConnector {
                table_name: "ok".into(),
            }),
            dialect: SqlDialect::DuckDb,
            profile_id: None,
        },
    );
    registry.insert(
        "snowflake",
        ConnectionEntry {
            connector: Box::new(FailingConnector {
                dialect: SqlDialect::Snowflake,
            }),
            dialect: SqlDialect::Snowflake,
            profile_id: None,
        },
    );
    let tools = DatabaseTools::with_registry(registry, 100, true, None);

    let res = tools
        .execute(
            "bounded_sql_query_all",
            serde_json::json!({"sql": "SELECT 1"}),
        )
        .await
        .expect("fan-out should succeed even when one database fails");

    let databases = res["databases"].as_array().expect("databases array");
    assert_eq!(databases.len(), 2);
    assert!(
        databases[0].get("result").is_some(),
        "the healthy database still returns a result"
    );
    assert!(
        databases[1].get("error").is_some(),
        "the failing database reports an error instead of sinking the run"
    );
    assert!(
        databases[1]["error"]
            .as_str()
            .is_some_and(|error| error.contains("syntax error near FROM")),
        "safe connector detail should identify the dialect mismatch"
    );
}

#[tokio::test]
async fn bounded_sql_query_all_runs_connections_concurrently() {
    let barrier = Arc::new(Barrier::new(2));
    let mut registry = ConnectionRegistry::new("primary");
    for name in ["primary", "warehouse"] {
        registry.insert(
            name,
            ConnectionEntry {
                connector: Box::new(BarrierConnector {
                    barrier: barrier.clone(),
                }),
                dialect: SqlDialect::DuckDb,
                profile_id: None,
            },
        );
    }
    let tools = DatabaseTools::with_registry(registry, 100, true, None);

    let result = tokio::time::timeout(
        Duration::from_secs(1),
        tools.execute(
            "bounded_sql_query_all",
            serde_json::json!({"sql": "SELECT 1"}),
        ),
    )
    .await;
    assert!(
        result.is_ok(),
        "both queries must start before either can finish"
    );
}
