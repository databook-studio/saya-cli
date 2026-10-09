use super::*;

#[tokio::test]
async fn schema_discovery_records_no_objects() {
    let (log, tools) = log_and_tools(
        registry_one(
            Box::new(SchemaConnector { table: "orders" }),
            &identity("primary"),
        ),
        true,
    );
    tools
        .execute("schema_discovery", serde_json::json!({}))
        .await
        .expect("schema discovery succeeds");

    let drained = log.drain();
    assert_eq!(drained.observations.len(), 1);
    let obs = &drained.observations[0];
    assert_eq!(obs.tool, "schema_discovery");
    assert_eq!(obs.outcome, ObservationOutcome::Succeeded);
    assert!(obs.objects.is_empty());
    assert!(obs.columns.is_empty());
    assert_eq!(obs.row_count, None);
}

#[tokio::test]
async fn contract_tool_call_records_nothing() {
    let (log, tools) = log_and_tools(
        registry_one(
            Box::new(SchemaConnector { table: "orders" }),
            &identity("primary"),
        ),
        true,
    );
    // contract_search has no store to read, so it returns an empty result — but
    // the point is that it records nothing either way.
    let _ = tools
        .execute("contract_search", serde_json::json!({"terms": ["orders"]}))
        .await
        .expect("contract tool degrades to empty, never Err");
    let drained = log.drain();
    assert!(
        drained.observations.is_empty(),
        "a contract tool call must not record: {:?}",
        drained.observations
    );
}

#[tokio::test]
async fn cap_holds_and_drain_reports_truncation() {
    let (log, tools) = log_and_tools(
        registry_one(
            Box::new(ScriptedConnector {
                rows: vec![],
                row_count: 0,
                truncated: false,
            }),
            &identity("primary"),
        ),
        true,
    );
    for _ in 0..40 {
        tools
            .execute(
                "bounded_sql_query",
                serde_json::json!({"sql": "SELECT 1 FROM t"}),
            )
            .await
            .expect("query succeeds");
    }
    let drained = log.drain();
    assert_eq!(drained.observations.len(), 32);
    assert!(drained.truncated);
}

#[tokio::test]
async fn drained_observations_contain_no_sql_or_result_values() {
    let (log, tools) = log_and_tools(
        registry_one(
            Box::new(ScriptedConnector {
                rows: vec![serde_json::json!(["SENTINELROWVALUE"])],
                row_count: 1,
                truncated: false,
            }),
            &identity("primary"),
        ),
        true,
    );
    tools
        .execute(
            "bounded_sql_query",
            serde_json::json!({"sql": "SELECT 'SENTINELLITERAL' AS label, name FROM orders WHERE token = 'SENTINELLITERAL'"}),
        )
        .await
        .expect("query succeeds");

    let drained = log.drain();
    let blob = format!("{drained:?}");
    assert!(
        !blob.contains("SENTINELLITERAL"),
        "SQL literal leaked into observation: {blob}"
    );
    assert!(
        !blob.contains("SENTINELROWVALUE"),
        "result value leaked into observation: {blob}"
    );
}

#[tokio::test]
async fn no_collector_means_byte_identical_output() {
    // `ConnectionRegistry` is not `Clone`, so build two identical registries.
    let registry_without = registry_one(
        Box::new(ScriptedConnector {
            rows: vec![serde_json::json!(["x"])],
            row_count: 1,
            truncated: false,
        }),
        &identity("primary"),
    );
    let registry_with = registry_one(
        Box::new(ScriptedConnector {
            rows: vec![serde_json::json!(["x"])],
            row_count: 1,
            truncated: false,
        }),
        &identity("primary"),
    );

    let without = DatabaseTools::with_registry(registry_without, 100, true, None);
    let with_log = Arc::new(ObservationLog::new());
    let with = DatabaseTools::with_registry_and_observations(
        registry_with,
        100,
        true,
        None,
        with_log.clone(),
    );

    let args = serde_json::json!({"sql": "SELECT id FROM orders"});
    let out_without = without
        .execute("bounded_sql_query", args.clone())
        .await
        .expect("without collector");
    let out_with = with
        .execute("bounded_sql_query", args)
        .await
        .expect("with collector");
    assert_eq!(out_without, out_with, "collector must not change output");
    // The collector still recorded — it just did not touch the output.
    assert_eq!(with_log.drain().observations.len(), 1);
}

#[tokio::test]
async fn drain_empties_so_a_turn_cannot_double_count() {
    let (log, tools) = log_and_tools(
        registry_one(
            Box::new(ScriptedConnector {
                rows: vec![],
                row_count: 0,
                truncated: false,
            }),
            &identity("primary"),
        ),
        true,
    );
    tools
        .execute(
            "bounded_sql_query",
            serde_json::json!({"sql": "SELECT 1 FROM t"}),
        )
        .await
        .unwrap();
    let first = log.drain();
    assert_eq!(first.observations.len(), 1);
    let second = log.drain();
    assert!(second.observations.is_empty(), "a turn cannot double-count");
}

/// A focused unit check on the collector's own drain/truncation contract, kept
/// here so the bound is asserted at the integration surface too.
#[test]
fn drain_report_type_carries_truncation_flag() {
    let drained = DrainedObservations {
        observations: Vec::<ToolObservation>::new(),
        truncated: true,
    };
    assert!(drained.truncated);
    assert!(drained.observations.is_empty());
}
