#[tokio::test]
async fn bounded_sql_query_all_respects_its_concurrency_cap() {
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
    let tools = DatabaseTools::with_registry_and_fan_out_limits(
        registry,
        100,
        true,
        1,
        Duration::from_secs(1),
    );

    assert!(
        tokio::time::timeout(
            Duration::from_millis(50),
            tools.execute(
                "bounded_sql_query_all",
                serde_json::json!({"sql": "SELECT 1"}),
            ),
        )
        .await
        .is_err(),
        "a cap of one must not start the second barrier participant"
    );
}

#[tokio::test]
async fn bounded_sql_query_all_reports_per_database_timeouts() {
    let mut registry = ConnectionRegistry::new("primary");
    registry.insert(
        "primary",
        ConnectionEntry {
            connector: Box::new(SlowConnector),
            dialect: SqlDialect::DuckDb,
            profile_id: None,
        },
    );
    let tools = DatabaseTools::with_registry_and_fan_out_limits(
        registry,
        100,
        true,
        1,
        Duration::from_millis(10),
    );

    let result = tools
        .execute(
            "bounded_sql_query_all",
            serde_json::json!({"sql": "SELECT 1"}),
        )
        .await
        .expect("fan-out timeout is reported per database");
    assert_eq!(result["databases"][0]["error"], "read-only query timed out");
}
