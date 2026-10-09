use super::*;

// -- the whole lane end to end (R3): tool execution → model shaping → TUI ---
// -- capture pairing. The connector is scripted here because this test must --
// -- reach both the agent tools (pub(crate)) and the private TUI pairing. ---

/// The 70,000-char-cell connector from the verified facts: the serialized
/// result crosses the loop's 65,536-byte message cap, so the model's view is
/// a truncated prefix without the second row.
struct SeventyKCellConnector;

#[async_trait::async_trait]
impl saya_connectors::DatabaseConnector for SeventyKCellConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::DuckDb
    }
    async fn connect(&self) -> Result<(), saya_types::ConnectionError> {
        Ok(())
    }
    async fn schema(&self) -> Result<saya_types::SchemaTree, saya_types::ConnectionError> {
        Ok(saya_types::SchemaTree::default())
    }
    async fn execute(
        &self,
        req: saya_types::QueryRequest,
    ) -> Result<QueryResult, saya_types::ConnectionError> {
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

/// A truncated model view is refused from `DatabaseTools` execution through
/// the production hook onto the turn's channel, into `drain_stream`'s
/// pairing: the slot stays empty, the gap records the reason, the completion
/// still promotes the selectable query, and the gap's message names it.
#[tokio::test]
async fn a_truncated_model_view_is_refused_from_tool_execution_to_the_drain_pairing() {
    use crate::agent::tools::DatabaseTools;
    use crate::interactive::session::state::SessionState;
    use crate::interactive::tui::agent::spawn::capture_hook;
    use crate::interactive::tui::agent::{Stream, StreamMsg};
    use crate::interactive::tui::application::tests_support::idle_app;
    use saya_agent::{AgentEvent, CancellationToken, ToolExecutor, shape_tool_result};

    // The production hook, built exactly as the TUI builds it, onto the
    // turn's stream channel.
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let events_tx = tx.clone();
    let hook = capture_hook(tx);
    let budget = saya_agent::AgentLimits::default().context_byte_budget;
    let tools = DatabaseTools::new(Some(Box::new(SeventyKCellConnector)), 100, true)
        .with_capture(Some(hook), budget);

    // The loop's message order: the request, then the tool runs (the hook
    // forwards during it), then the completion promotes.
    let _ = events_tx.send(StreamMsg::Event(AgentEvent::tool_requested(
        "bounded_sql_query",
        serde_json::json!({"sql": "SELECT big", "connection": "primary"}),
        None,
    )));
    let value = tools
        .execute(
            "bounded_sql_query",
            serde_json::json!({"sql": "SELECT big", "connection": "primary"}),
        )
        .await
        .expect("the model still receives its full result");
    let _ = events_tx.send(StreamMsg::Event(AgentEvent::ToolCompleted {
        name: "bounded_sql_query".into(),
        summary: "2 rows".into(),
    }));

    // The shaping step: the loop's own shaping, at the loop's own budget,
    // says the model view was cut — without the second row.
    let shaped = shape_tool_result(&value, budget);
    assert!(
        shaped.truncated,
        "a {}-byte serialization exceeds the {}-byte message cap",
        serde_json::to_string(&value).unwrap().len(),
        saya_agent::tool_message_cap(budget)
    );
    assert!(
        !shaped.text.contains("second-row-sentinel"),
        "the model saw a truncated prefix without the second row"
    );

    // The drain pairing, driven through `drain_stream` — the entry point the
    // loop tick uses.
    let (mut app, mut state) = (idle_app(), SessionState::new("s1", None, "test-model"));
    app.request.stream = Some(Stream {
        rx,
        cancel: CancellationToken::new(),
        prompt: "question".into(),
        question_scope: None,
    });
    app.drain_stream(&mut state);
    assert!(
        app.captured.is_none(),
        "nothing is held: the model saw a cut prefix, so there is no capture"
    );
    assert_eq!(
        app.agent_captures.gap,
        Some(CaptureGap::ModelViewTruncated),
        "the drain pairing records the refusal's reason"
    );
    assert_eq!(
        app.last_query.as_ref().map(|q| q.sql.as_str()),
        Some("SELECT big"),
        "the successful completion still promotes the selectable query"
    );
    assert_eq!(
        app.agent_captures.gap.map(super::CaptureGap::message),
        Some(
            "The agent received a truncated version of this result, so it was \
             not captured. Use /export --refresh to re-run it."
                .to_owned()
        ),
        "the snapshot refusal names the reason"
    );
}
