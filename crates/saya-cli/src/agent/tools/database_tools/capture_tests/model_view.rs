use super::*;

// -- the model-view gate (R3): a capture is the agent's evidence only when --
// -- it is exactly what the model received. `shape_tool_result` — the ------
// -- loop's own shaping, at the loop's own budget — decides. ----------------

/// A connector returning one 70,000-char first cell plus a sentinel second
/// row: the serialized result crosses the loop's 65,536-byte message cap, so
/// the model's view is a truncated prefix without the second row — while the
/// typed result stays far under the accounted capture budget.
struct SeventyKCellConnector;

#[async_trait]
impl DatabaseConnector for SeventyKCellConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::DuckDb
    }
    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }
    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(SchemaTree::default())
    }
    async fn execute(&self, req: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Ok(QueryResult {
            columns: vec!["big".into()],
            rows: vec![
                serde_json::json!(["x".repeat(70_000)]),
                serde_json::json!(["second-row-sentinel"]),
            ],
            row_count: 2,
            truncated: false,
            executed_sql: req.sql,
        })
    }
}

/// A connector whose single cell is credential-shaped: the loop redacts it
/// before the message reaches the model, so a raw capture is not what the
/// model saw.
struct SecretCellConnector;

#[async_trait]
impl DatabaseConnector for SecretCellConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::DuckDb
    }
    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }
    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(SchemaTree::default())
    }
    async fn execute(&self, req: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Ok(QueryResult {
            columns: vec!["secret".into()],
            rows: vec![serde_json::json!(["token=abc123"])],
            row_count: 1,
            truncated: false,
            executed_sql: req.sql,
        })
    }
}

/// A large first cell puts the serialized result past the loop's message cap:
/// the model received a truncated prefix (without the second row), so the
/// capture is refused as truncated — never held as the model's evidence.
#[tokio::test]
async fn large_cell_capture_is_refused_as_truncated() {
    let (hook, observed) = observing_hook();
    let budget = loop_budget();
    let tools = DatabaseTools::new(Some(Box::new(SeventyKCellConnector)), 100, true)
        .with_capture(Some(hook), budget);
    let value = tools
        .execute(
            "bounded_sql_query",
            serde_json::json!({"sql": "SELECT big", "connection": "primary"}),
        )
        .await
        .expect("the model still receives its full result");
    // The same shaping the loop applies, at the same budget, says the model's
    // view was cut — and the cut prefix lacks the second row the typed result
    // carries.
    let shaped = shape_tool_result(&value, budget);
    assert!(
        shaped.truncated,
        "a {}-byte serialization exceeds the {}-byte message cap",
        serde_json::to_string(&value).unwrap().len(),
        saya_agent::tool_message_cap(budget)
    );
    assert_eq!(shaped.redactions, 0, "nothing was redacted here");
    assert!(
        !shaped.text.contains("second-row-sentinel"),
        "the model saw a truncated prefix without the second row: {} bytes",
        shaped.text.len()
    );
    let events = observed.lock().unwrap();
    assert_eq!(events.len(), 1, "exactly one event: {events:?}");
    assert!(
        matches!(
            &events[0],
            CaptureEvent::Refused { sql, connection, reason: CaptureRefusalReason::ModelViewTruncated }
                if sql == "SELECT big" && connection == "primary"
        ),
        "a truncated model view is refused as truncated: {events:?}"
    );
}

/// A credential-shaped cell is redacted before the message reaches the model:
/// the model never saw the raw value, so the capture is refused as redacted —
/// the raw result must not become evidence labelled as the model's.
#[tokio::test]
async fn credential_shaped_cell_capture_is_refused_as_redacted() {
    let (hook, observed) = observing_hook();
    let budget = loop_budget();
    let tools = DatabaseTools::new(Some(Box::new(SecretCellConnector)), 100, true)
        .with_capture(Some(hook), budget);
    let value = tools
        .execute(
            "bounded_sql_query",
            serde_json::json!({"sql": "SELECT secret", "connection": "primary"}),
        )
        .await
        .expect("the model still receives its (redacted) result");
    // The tool value keeps the raw cell (the loop redacts at its own message
    // boundary), and the shaping confirms the model's view was redacted —
    // without truncation.
    let shaped = shape_tool_result(&value, budget);
    assert_eq!(
        value["rows"][0][0], "token=abc123",
        "the tool value is unchanged: redaction is the loop's boundary"
    );
    assert_eq!(shaped.redactions, 1, "the credential shape was replaced");
    assert!(
        !shaped.truncated,
        "the result fits the message cap whole: {} bytes",
        shaped.text.len()
    );
    assert!(
        shaped.text.contains("[redacted]") && !shaped.text.contains("token=abc123"),
        "the model's view is the masked form, not the raw value"
    );
    let events = observed.lock().unwrap();
    assert_eq!(events.len(), 1, "exactly one event: {events:?}");
    assert!(
        matches!(
            &events[0],
            CaptureEvent::Refused { sql, connection, reason: CaptureRefusalReason::ModelViewRedacted }
                if sql == "SELECT secret" && connection == "primary"
        ),
        "a redacted model view is refused as redacted: {events:?}"
    );
}

/// A small, clean result passes the model-view gate: captured, and the
/// shaping confirms the model received exactly these bytes unchanged.
#[tokio::test]
async fn small_clean_result_is_captured_as_model_visible() {
    let (hook, observed) = observing_hook();
    let budget = loop_budget();
    let tools = DatabaseTools::new(
        Some(Box::new(ScriptedConnector {
            rows: vec![serde_json::json!(["x"]), serde_json::json!(["y"])],
            row_count: 2,
            truncated: false,
        })),
        100,
        true,
    )
    .with_capture(Some(hook), budget);
    let value = tools
        .execute(
            "bounded_sql_query",
            serde_json::json!({"sql": "SELECT c", "connection": "primary"}),
        )
        .await
        .expect("the query succeeds");
    let shaped = shape_tool_result(&value, budget);
    assert!(
        !shaped.truncated && shaped.redactions == 0,
        "the model received this result unchanged: truncated={} redactions={}",
        shaped.truncated,
        shaped.redactions
    );
    let events = observed.lock().unwrap();
    assert_eq!(events.len(), 1, "exactly one event: {events:?}");
    let CaptureEvent::Captured(capture) = &events[0] else {
        panic!("a clean result is captured, got {:?}", events[0]);
    };
    let model_result: QueryResult = serde_json::from_value(value).unwrap();
    assert_eq!(
        capture.result, model_result,
        "the capture is exactly the model-visible result"
    );
}
