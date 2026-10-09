#[tokio::test]
async fn bounded_sql_query_all_is_blocked_when_data_sharing_is_disabled() {
    let tools = DatabaseTools::with_registry(two_connection_registry(), 100, false, None);
    let err = tools
        .execute(
            "bounded_sql_query_all",
            serde_json::json!({"sql": "SELECT 1"}),
        )
        .await
        .expect_err("fan-out must respect the data-sharing guard");
    assert!(
        err.to_string().contains("data sharing is disabled"),
        "got: {err}"
    );
}

#[tokio::test]
async fn tool_execution_rejects_arguments_outside_its_schema() {
    let tools = DatabaseTools::new(None, 100, true);
    for (name, arguments) in [
        ("schema_discovery", serde_json::json!({"sql": "SELECT 1"})),
        ("bounded_sql_query", serde_json::json!({"sql": 1})),
        (
            "bounded_sql_query_all",
            serde_json::json!({"sql": "SELECT 1", "connection": "primary"}),
        ),
    ] {
        let error = tools
            .execute(name, arguments)
            .await
            .expect_err("invalid tool arguments must not reach a connector");
        assert!(
            error.to_string().contains("invalid tool arguments"),
            "got: {error}"
        );
    }
}

#[test]
fn definitions_include_fan_out_only_when_query_data_allowed() {
    let with_data: Vec<String> = DatabaseTools::definitions(true, false, false, false, true)
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    assert!(with_data.iter().any(|name| name == "bounded_sql_query_all"));

    let without_data: Vec<String> = DatabaseTools::definitions(false, false, false, false, false)
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    assert!(
        !without_data
            .iter()
            .any(|name| name == "bounded_sql_query_all")
    );
    assert!(!without_data.iter().any(|name| name == "bounded_sql_query"));
}

#[test]
fn definitions_preserve_the_read_only_and_approval_contract() {
    let tools = DatabaseTools::definitions(true, false, false, false, true);
    let tool = |name: &str| tools.iter().find(|tool| tool.name == name).unwrap();

    let schema = tool("schema_discovery");
    assert!(schema.read_only);
    assert!(!schema.effect.requires_approval);
    assert!(schema.parameters["properties"]["connection"].is_object());
    assert!(schema.parameters.get("required").is_none());

    let single = tool("bounded_sql_query");
    assert!(single.read_only);
    assert!(single.effect.requires_approval);
    assert_eq!(single.parameters["required"], serde_json::json!(["sql"]));
    assert!(single.parameters["properties"]["connection"].is_object());
    assert!(single.parameters["properties"]["sql"].is_object());

    let all = tool("bounded_sql_query_all");
    assert!(all.read_only);
    assert!(all.effect.requires_approval);
    assert_eq!(all.parameters["required"], serde_json::json!(["sql"]));
    assert!(all.parameters["properties"]["sql"].is_object());
    assert!(all.parameters["properties"].get("connection").is_none());
}
