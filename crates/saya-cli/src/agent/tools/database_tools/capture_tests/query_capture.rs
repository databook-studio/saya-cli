use super::*;

/// One successful `bounded_sql_query`: the hook receives the typed result the
/// model saw, under the model row cap — the same rows, byte-for-byte.
#[tokio::test]
async fn agent_query_hook_receives_the_rows_the_model_saw() {
    let (connector, dir) = sqlite_fixture().await;
    let (hook, observed) = observing_hook();
    let tools =
        DatabaseTools::new(Some(connector), 200, true).with_capture(Some(hook), loop_budget());
    let value = tools
        .execute(
            "bounded_sql_query",
            serde_json::json!({"sql": "SELECT id, name FROM users ORDER BY id"}),
        )
        .await
        .expect("query succeeds");

    let events = observed.lock().unwrap();
    assert_eq!(events.len(), 1, "exactly one capture event: {events:?}");
    let CaptureEvent::Captured(capture) = &events[0] else {
        panic!("expected a capture, got {:?}", events[0]);
    };
    // The hook's typed result is exactly what the model received.
    let model_result: QueryResult = serde_json::from_value(value).unwrap();
    assert_eq!(capture.result, model_result);
    // The applied cap is the model cap, not the configured 200 — and the
    // 60-row table is capped to it.
    assert_eq!(
        capture.row_cap,
        crate::agent::state_tools::model_row_cap(200)
    );
    assert_eq!(capture.result.rows.len(), 50);
    assert!(capture.result.truncated);
    assert!(capture.started_unix_ms <= capture.finished_unix_ms);
    let _ = std::fs::remove_dir_all(dir);
}

/// When the model omits `connection` (or passes an empty one) the registry
/// resolves the primary: the capture records that resolved name, plus the
/// entry's dialect and profile identity — never the omission.
#[tokio::test]
async fn omitted_connection_records_the_resolved_connection_name() {
    let (hook, observed) = observing_hook();
    let mut registry = ConnectionRegistry::new("warehouse");
    registry.insert("warehouse", scripted_entry());
    let tools = DatabaseTools::with_registry(registry, 100, true, None)
        .with_capture(Some(hook), loop_budget());
    for connection in [None, Some(""), Some("warehouse")] {
        let mut arguments = serde_json::json!({"sql": "SELECT 1"});
        if let Some(connection) = connection {
            arguments["connection"] = serde_json::json!(connection);
        }
        tools
            .execute("bounded_sql_query", arguments)
            .await
            .expect("query succeeds");
    }
    let events = observed.lock().unwrap();
    assert_eq!(events.len(), 3);
    for event in events.iter() {
        let CaptureEvent::Captured(capture) = event else {
            panic!("expected a capture, got {event:?}");
        };
        assert_eq!(capture.connection, "warehouse");
        assert_eq!(
            capture.profile_identity.as_deref(),
            Some("p-capture-identity-sentinel")
        );
        assert_eq!(capture.dialect, SqlDialect::Sqlite);
    }
}

/// Fan-out and the probe tools run SQL through the same state-tools path but
/// must never capture: only single-connection `bounded_sql_query` does.
#[tokio::test]
async fn fanout_and_probe_tools_never_capture() {
    let (hook, observed) = observing_hook();
    let tools = DatabaseTools::new(
        Some(Box::new(ScriptedConnector {
            rows: vec![serde_json::json!(["x"]), serde_json::json!(["y"])],
            row_count: 2,
            truncated: false,
        })),
        100,
        true,
    )
    .with_capture(Some(hook), loop_budget());
    tools
        .execute(
            "bounded_sql_query_all",
            serde_json::json!({"sql": "SELECT 1"}),
        )
        .await
        .expect("fan-out succeeds");
    tools
        .execute("schema_discovery", serde_json::json!({}))
        .await
        .expect("schema discovery succeeds");
    tools
        .execute("result_shape", serde_json::json!({"sql": "SELECT 1"}))
        .await
        .expect("result shape succeeds");
    tools
        .execute("column_health", serde_json::json!({"sql": "SELECT 1"}))
        .await
        .expect("column health succeeds");
    // A probeable statement, so join_check's two COUNT queries really run.
    tools
        .execute(
            "join_check",
            serde_json::json!({"sql": "SELECT SUM(o.amount) FROM orders o \
                 JOIN items i ON o.id = i.order_id"}),
        )
        .await
        .expect("join check succeeds");
    assert!(
        observed.lock().unwrap().is_empty(),
        "only bounded_sql_query captures"
    );
}

/// A failed query captures nothing: there is no result the model saw.
#[tokio::test]
async fn failed_query_never_captures() {
    let (hook, observed) = observing_hook();
    let tools = DatabaseTools::new(Some(Box::new(FailingConnector)), 100, true)
        .with_capture(Some(hook), loop_budget());
    let error = tools
        .execute("bounded_sql_query", serde_json::json!({"sql": "SELECT 1"}))
        .await
        .expect_err("a connector failure surfaces as a tool Err");
    let _ = error;
    assert!(
        observed.lock().unwrap().is_empty(),
        "a failure never captures"
    );
}

/// A cancellation visible after query settlement but before capture handoff
/// keeps the successful rows out of the TUI evidence stream.
#[test]
fn cancelled_capture_handoff_emits_no_event() {
    let (hook, observed) = observing_hook();
    let tools = DatabaseTools::new(None, 100, true).with_capture(Some(hook), loop_budget());
    let entry = scripted_entry();
    let executed = ExecutedQuery {
        value: serde_json::json!({}),
        result: QueryResult {
            columns: vec!["c".into()],
            rows: vec![serde_json::json!(["value"])],
            row_count: 1,
            truncated: false,
            executed_sql: "SELECT 'value'".into(),
        },
        row_cap: 1,
        started_unix_ms: 1,
        finished_unix_ms: 2,
    };
    let cancellation = saya_agent::CancellationToken::new();
    cancellation.cancel();
    let context = saya_agent::ToolExecutionContext {
        result_cap: loop_budget(),
        cancellation,
    };

    tools.emit_query_capture(
        &serde_json::json!({"sql": "SELECT 'value'"}),
        "SELECT 'value'",
        &entry,
        &executed,
        &context,
    );

    assert!(
        observed.lock().unwrap().is_empty(),
        "cancelled rows are not captured"
    );
}

/// Without a hook the executor behaves exactly as before: the same query
/// returns the same value with and without one attached.
#[tokio::test]
async fn no_hook_no_behaviour_change() {
    let query = serde_json::json!({"sql": "SELECT id FROM orders"});
    let script = ScriptedConnector {
        rows: vec![serde_json::json!(["x"])],
        row_count: 1,
        truncated: false,
    };
    let plain = DatabaseTools::new(Some(Box::new(script)), 100, true);
    let plain_value = plain
        .execute("bounded_sql_query", query.clone())
        .await
        .expect("plain path succeeds");
    let (hook, observed) = observing_hook();
    let hooked = DatabaseTools::new(
        Some(Box::new(ScriptedConnector {
            rows: vec![serde_json::json!(["x"])],
            row_count: 1,
            truncated: false,
        })),
        100,
        true,
    )
    .with_capture(Some(hook), loop_budget());
    let hooked_value = hooked
        .execute("bounded_sql_query", query)
        .await
        .expect("hooked path succeeds");
    assert_eq!(plain_value, hooked_value, "the model's value is identical");
    assert_eq!(observed.lock().unwrap().len(), 1);
}
