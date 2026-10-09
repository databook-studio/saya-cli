#[cfg(unix)]
#[tokio::test]
async fn render_chart_creates_0600_permissions_file() {
    let _guard = crate::chart::test_support::lock_session_charts_for_test();
    use async_trait::async_trait;
    use saya_connectors::DatabaseConnector;
    use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};

    struct NonEmptyConnector;

    #[async_trait]
    impl DatabaseConnector for NonEmptyConnector {
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
                columns: vec!["cat".into(), "val".into()],
                rows: vec![serde_json::json!(["A", 10])],
                row_count: 1,
                truncated: false,
                executed_sql: req.sql,
            })
        }
    }

    let tools = DatabaseTools::new(Some(Box::new(NonEmptyConnector)), 100, true);
    let res = tools
        .execute(
            "render_chart",
            serde_json::json!({"sql": "SELECT 1", "chart_type": "bar"}),
        )
        .await
        .expect("render_chart should succeed");

    let path_str = res["path"].as_str().expect("path in response");
    let path = std::path::Path::new(path_str);
    assert_eq!(path.parent(), Some(std::env::temp_dir().as_path()));
    assert!(
        path.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("saya-chart-")
    );
    let meta = std::fs::metadata(path).expect("file should exist");

    use std::os::unix::fs::PermissionsExt;
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    let _ = std::fs::remove_file(path);
}

#[cfg(unix)]
#[tokio::test]
async fn render_chart_saves_contained_html_without_returning_rows() {
    let _guard = crate::chart::test_support::lock_session_charts_for_test();
    use async_trait::async_trait;
    use saya_connectors::DatabaseConnector;
    use saya_harness::workspace::Workspace;
    use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};

    struct NonEmptyConnector;

    #[async_trait]
    impl DatabaseConnector for NonEmptyConnector {
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
                columns: vec!["cat".into(), "val".into()],
                rows: vec![serde_json::json!(["A", 10])],
                row_count: 1,
                truncated: false,
                executed_sql: req.sql,
            })
        }
    }

    let root = std::env::temp_dir().join(format!(
        "saya-chart-save-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after epoch")
            .as_nanos()
    ));
    std::fs::create_dir(&root).expect("workspace tempdir");
    let mut tools = DatabaseTools::new(Some(Box::new(NonEmptyConnector)), 100, true);
    tools.workspace = Some(Arc::new(Workspace::open(&root).expect("workspace opens")));
    let res = tools
        .execute(
            "render_chart",
            serde_json::json!({
                "sql": "SELECT 1",
                "chart_type": "bar",
                "save_to": "monthly.html"
            }),
        )
        .await
        .expect("render_chart should save");

    assert_eq!(res["path"], "monthly.html");
    assert_eq!(res.as_object().unwrap().len(), 2);
    let saved = root.join("monthly.html");
    let bytes = std::fs::read(&saved).expect("contained chart exists");
    let result = QueryResult {
        columns: vec!["cat".into(), "val".into()],
        rows: vec![serde_json::json!(["A", 10])],
        row_count: 1,
        truncated: false,
        executed_sql: "SELECT 1".into(),
    };
    let mut spec = crate::chart::suggest_spec(&result);
    spec.kind = crate::chart::ChartKind::Bar;
    let expected = crate::chart::render_html(&result, &spec).expect("chart renders");
    assert_eq!(bytes, expected.as_bytes());

    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(saved).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let outside = root
        .parent()
        .unwrap()
        .join(format!("saya-chart-outside-{}", std::process::id()));
    std::fs::write(&outside, b"sentinel").expect("outside sentinel");
    let paths = [
        format!("../{}", outside.file_name().unwrap().to_string_lossy()),
        outside.display().to_string(),
    ];
    for path in &paths {
        let error = tools
            .execute(
                "render_chart",
                serde_json::json!({
                    "sql": "SELECT 1",
                    "chart_type": "bar",
                    "save_to": path
                }),
            )
            .await
            .expect_err("escaping chart path must refuse");
        assert!(matches!(error, saya_agent::ToolError::WorkspaceWrite(_)));
    }
    assert_eq!(std::fs::read(&outside).unwrap(), b"sentinel");
    let symlink = root.join("linked.html");
    std::os::unix::fs::symlink(&outside, &symlink).expect("workspace symlink");
    let error = tools
        .execute(
            "render_chart",
            serde_json::json!({
                "sql": "SELECT 1",
                "chart_type": "bar",
                "save_to": "linked.html"
            }),
        )
        .await
        .expect_err("symlink chart path must refuse");
    assert!(matches!(error, saya_agent::ToolError::WorkspaceWrite(_)));
    assert_eq!(std::fs::read(&outside).unwrap(), b"sentinel");
    std::fs::remove_file(outside).expect("outside sentinel cleans up");
    tools.workspace = None;
    let error = tools
        .execute(
            "render_chart",
            serde_json::json!({
                "sql": "SELECT 1",
                "chart_type": "bar",
                "save_to": "missing-workspace.html"
            }),
        )
        .await
        .expect_err("saving without a bound workspace must refuse");
    assert_eq!(error, saya_agent::ToolError::WorkspaceUnavailable);
    std::fs::remove_dir_all(root).expect("workspace cleans up");
}
