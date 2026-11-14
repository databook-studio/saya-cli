use super::*;

#[tokio::test]
async fn successful_query_records_object_column_and_row_count() {
    let (log, tools) = log_and_tools(
        registry_one(
            Box::new(ScriptedConnector {
                rows: vec![serde_json::json!(["x"]), serde_json::json!(["y"])],
                row_count: 2,
                truncated: false,
            }),
            &identity("primary"),
        ),
        true,
    );
    tools
        .execute(
            "bounded_sql_query",
            serde_json::json!({"sql": "SELECT id FROM analytics.public.orders"}),
        )
        .await
        .expect("query succeeds");

    let drained = log.drain();
    assert_eq!(drained.observations.len(), 1);
    let obs = &drained.observations[0];
    assert_eq!(obs.tool, "bounded_sql_query");
    assert_eq!(obs.outcome, ObservationOutcome::Succeeded);
    assert_eq!(obs.objects, vec![vec!["analytics", "public", "orders"]]);
    assert_eq!(obs.columns, vec!["id".to_string()]);
    assert_eq!(obs.row_count, Some(2));
    assert_eq!(obs.truncated, Some(false));
    assert!(!obs.references_partial);
    assert_eq!(obs.profile, Some(identity("primary")));
}

#[tokio::test]
async fn failed_query_records_failed_without_error_text() {
    let (log, tools) = log_and_tools(
        registry_one(Box::new(FailingConnector), &identity("primary")),
        true,
    );
    let err = tools
        .execute(
            "bounded_sql_query",
            serde_json::json!({"sql": "SELECT id FROM orders"}),
        )
        .await
        .expect_err("a connector failure surfaces as a tool Err");
    // The tool error string is NOT recorded — only the outcome enum is.
    let _ = err;

    let drained = log.drain();
    assert_eq!(drained.observations.len(), 1);
    let obs = &drained.observations[0];
    assert_eq!(obs.outcome, ObservationOutcome::Failed);
    // The observation carries no error text anywhere — and the connector error
    // string itself contained a sentinel that must not ride along.
    let blob = format!("{obs:?}");
    assert!(!blob.contains("syntax error"), "error text leaked: {blob}");
    assert_eq!(obs.row_count, None);
    assert_eq!(obs.truncated, None);
    // Failed query still records the objects the SQL named (it parsed).
    assert_eq!(obs.objects, vec![vec!["orders"]]);
}

#[tokio::test]
async fn denied_call_records_denied_with_no_objects_or_rows() {
    let (log, tools) = log_and_tools(
        registry_one(
            Box::new(ScriptedConnector {
                rows: vec![],
                row_count: 0,
                truncated: false,
            }),
            &identity("primary"),
        ),
        false,
    );
    let err = tools
        .execute(
            "bounded_sql_query",
            serde_json::json!({"sql": "SELECT id FROM orders"}),
        )
        .await
        .expect_err("data-sharing-disabled refuses the tool as Err");
    // The tool error string is not recorded — only the Denied outcome is.
    let _ = err;

    let drained = log.drain();
    assert_eq!(drained.observations.len(), 1);
    let obs = &drained.observations[0];
    assert_eq!(obs.outcome, ObservationOutcome::Denied);
    assert!(obs.objects.is_empty());
    assert!(obs.columns.is_empty());
    assert_eq!(obs.row_count, None);
    assert_eq!(obs.truncated, None);
    assert_eq!(obs.profile, None);
}

#[tokio::test]
async fn fan_out_records_one_observation_per_connection() {
    let primary = identity("primary");
    let warehouse = identity("warehouse");
    let mut registry = ConnectionRegistry::new("primary");
    registry.insert(
        "primary",
        entry_with(
            Box::new(ScriptedConnector {
                rows: vec![],
                row_count: 0,
                truncated: false,
            }),
            &primary,
        ),
    );
    registry.insert(
        "warehouse",
        ConnectionEntry {
            connector: Box::new(ScriptedConnector {
                rows: vec![],
                row_count: 0,
                truncated: false,
            }),
            dialect: SqlDialect::Postgres,
            profile_id: Some(warehouse.as_str().to_string()),
        },
    );
    let (log, tools) = log_and_tools(registry, true);
    tools
        .execute(
            "bounded_sql_query_all",
            serde_json::json!({"sql": "SELECT id FROM orders"}),
        )
        .await
        .expect("fan-out succeeds");

    let drained = log.drain();
    assert_eq!(drained.observations.len(), 2);
    let profiles: Vec<_> = drained.observations.iter().map(|o| &o.profile).collect();
    assert!(profiles.contains(&&Some(primary.clone())));
    assert!(profiles.contains(&&Some(warehouse.clone())));
    assert_ne!(
        profiles[0], profiles[1],
        "each observation has its own profile"
    );
}
