#[tokio::test]
async fn render_chart_over_bound_does_not_replace_saved_file() {
    use async_trait::async_trait;
    use saya_connectors::DatabaseConnector;
    use saya_harness::workspace::{MAX_IO_BYTES, Workspace};
    use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};

    struct LargeConnector;
    #[async_trait]
    impl DatabaseConnector for LargeConnector {
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
                columns: vec!["large".into()],
                rows: vec![serde_json::json!(["x".repeat(MAX_IO_BYTES + 1)])],
                row_count: 1,
                truncated: false,
                executed_sql: req.sql,
            })
        }
    }

    let root = std::env::temp_dir().join(format!("saya-chart-bound-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir(&root).expect("workspace tempdir");
    let destination = root.join("chart.html");
    std::fs::write(&destination, b"old chart").expect("existing destination");
    let mut tools = DatabaseTools::new(Some(Box::new(LargeConnector)), 100, true);
    tools.workspace = Some(Arc::new(Workspace::open(&root).expect("workspace opens")));

    let error = tools
        .execute(
            "render_chart",
            serde_json::json!({
                "sql": "SELECT large",
                "chart_type": "bar",
                "save_to": "chart.html"
            }),
        )
        .await
        .expect_err("over-bound HTML must refuse");
    assert!(matches!(error, saya_agent::ToolError::WorkspaceWrite(_)));
    assert_eq!(std::fs::read(destination).unwrap(), b"old chart");
    std::fs::remove_dir_all(root).expect("workspace cleans up");
}
