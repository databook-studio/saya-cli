/// A connector that answers one row, so the turn's database call produces
/// observable shape.
struct RowConnector;

#[async_trait]
impl saya_connectors::DatabaseConnector for RowConnector {
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
            columns: vec!["count".into()],
            rows: vec![serde_json::json!([42])],
            row_count: 1,
            truncated: false,
            executed_sql: req.sql,
        })
    }
}

/// The ask path's user: the engine resolves the call, and an ask is answered
/// "y" — exactly the shape the terminal prompt and the TUI modal render, so
/// the engine (not the stub) owns the decision shape.
struct AskYesDecider;

#[async_trait]
impl saya_agent::ApprovalDecider for AskYesDecider {
    async fn approve(&self, tool: &saya_agent::ToolDefinition, _: &serde_json::Value) -> bool {
        use saya_agent::{ApprovalDecision, SessionPolicy};
        match SessionPolicy::new(saya_agent::ApprovalPolicy::Ask).resolve(&tool.effect, None) {
            ApprovalDecision::Allow => true,
            ApprovalDecision::Ask => true,
            ApprovalDecision::Deny { reason: _ } => false,
        }
    }
}

/// Three tool calls in sequence, then the answer — one turn, three asks.
struct ThreeCallsProvider;

#[async_trait]
impl ChatProvider for ThreeCallsProvider {
    fn name(&self) -> &str {
        "three-calls"
    }
    async fn complete(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("stream path is used")
    }
    async fn stream(
        &self,
        _: ChatRequest,
        _: saya_agent::CancellationToken,
    ) -> Result<saya_agent::ProviderStream, ProviderError> {
        static CALL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = CALL.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let events = match n {
            0 => vec![
                Ok(saya_agent::ProviderEvent::ToolCalls(vec![ToolCall {
                    id: "c1".into(),
                    name: "bounded_sql_query".into(),
                    arguments: serde_json::json!({"sql": "SELECT count(*) FROM orders"}),
                }])),
                Ok(saya_agent::ProviderEvent::Done),
            ],
            1 => vec![
                Ok(saya_agent::ProviderEvent::ToolCalls(vec![ToolCall {
                    id: "c2".into(),
                    name: "workspace_write".into(),
                    arguments: serde_json::json!({"path": "src/notes.md", "content": "written"}),
                }])),
                Ok(saya_agent::ProviderEvent::Done),
            ],
            2 => vec![
                Ok(saya_agent::ProviderEvent::ToolCalls(vec![ToolCall {
                    id: "c3".into(),
                    name: "scratch_sql".into(),
                    arguments: serde_json::json!({"sql": "CREATE TABLE staged AS SELECT 1 AS one"}),
                }])),
                Ok(saya_agent::ProviderEvent::Done),
            ],
            _ => vec![
                Ok(saya_agent::ProviderEvent::TextDelta("done".into())),
                Ok(saya_agent::ProviderEvent::Done),
            ],
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}
