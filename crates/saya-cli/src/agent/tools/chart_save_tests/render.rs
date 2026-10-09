use super::*;

/// A present non-null `expected_digest` must be a string, surfaced through
/// the same typed error `workspace_write` uses.
#[tokio::test]
async fn render_chart_rejects_a_non_string_expected_digest() {
    let tools = crate::agent::tools::DatabaseTools::with_registry(
        crate::connection::ConnectionRegistry::new("primary"),
        100,
        true,
        None,
    );
    let error = tools
        .execute(
            "render_chart",
            serde_json::json!({"sql": "SELECT 1", "chart_type": "bar", "expected_digest": 7}),
        )
        .await
        .expect_err("a non-string digest must be rejected at validation");
    assert_eq!(error, ToolError::ExpectedDigestNotString);
}

/// An explicit null digest counts as absent, not mistyped: validation passes
/// it, and the call proceeds to the workspace check (which fails first on a
/// workspace-less run — a mistyped digest would have been rejected earlier
/// with `ExpectedDigestNotString`, and the pre-change schema rejected the
/// key outright). Dispatch resolves a connection before the tool's own
/// workspace check, so a synthetic connector reaches it.
#[tokio::test]
async fn render_chart_treats_an_explicit_null_digest_as_absent() {
    use saya_connectors::DatabaseConnector;
    use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};

    struct NonEmptyConnector;

    #[async_trait::async_trait]
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

    let tools =
        crate::agent::tools::DatabaseTools::new(Some(Box::new(NonEmptyConnector)), 100, true);
    let error = tools
        .execute(
            "render_chart",
            serde_json::json!({
                "sql": "SELECT 1",
                "chart_type": "bar",
                "save_to": "chart.html",
                "expected_digest": null
            }),
        )
        .await
        .expect_err("a null digest is the create form, but no workspace is bound here");
    assert_eq!(error, ToolError::WorkspaceUnavailable);
}

/// A digest without `save_to` refuses at validation: there is no destination
/// to guard, so the digest is a caller error, not an ignorable argument.
#[tokio::test]
async fn render_chart_rejects_expected_digest_without_save_to() {
    let tools = crate::agent::tools::DatabaseTools::with_registry(
        crate::connection::ConnectionRegistry::new("primary"),
        100,
        true,
        None,
    );
    let error = tools
        .execute(
            "render_chart",
            serde_json::json!({
                "sql": "SELECT 1",
                "chart_type": "bar",
                "expected_digest": "a".repeat(64)
            }),
        )
        .await
        .expect_err("a digest without save_to must refuse at validation");
    assert_eq!(
        error,
        ToolError::WorkspaceWrite("expected_digest is only valid together with save_to".into())
    );
}

/// The schema advertises the optional string `expected_digest` exactly when
/// it advertises `save_to`, the required list unchanged.
#[test]
fn render_chart_schema_advertises_expected_digest_only_with_the_save_permit() {
    let with_save = crate::agent::tools::DatabaseTools::definitions_with_chart_save(
        true, false, false, true, true, true,
    );
    let chart = with_save
        .iter()
        .find(|tool| tool.name == "render_chart")
        .expect("render_chart is advertised");
    let properties = chart.parameters["properties"]
        .as_object()
        .expect("the schema has properties");
    let declared = properties
        .get("expected_digest")
        .expect("expected_digest is declared");
    assert_eq!(declared["type"], "string", "the digest is a string");
    let description = declared["description"].as_str().expect("described");
    assert!(
        description.contains("workspace_read"),
        "the description says where the digest comes from: {description}"
    );
    assert!(
        description.contains("never invent"),
        "the description forbids invented digests: {description}"
    );
    assert_eq!(
        chart.parameters["required"],
        serde_json::json!(["sql", "chart_type"]),
        "the required list is unchanged: the digest stays optional"
    );

    let without_save = crate::agent::tools::DatabaseTools::definitions_with_chart_save(
        true, false, false, false, true, false,
    );
    let chart = without_save
        .iter()
        .find(|tool| tool.name == "render_chart")
        .expect("render_chart is advertised");
    assert!(
        chart.parameters["properties"]
            .get("expected_digest")
            .is_none(),
        "no digest argument without the save permit"
    );
}

/// Through the real tool: `render_chart` refuses to overwrite an existing
/// workspace file without a digest and leaves the bytes unchanged. The
/// refusal happens before the private temp copy, so the call returns before
/// anything is opened.
#[tokio::test]
async fn render_chart_refuses_to_overwrite_an_existing_file_without_a_digest() {
    use saya_connectors::DatabaseConnector;
    use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};

    struct NonEmptyConnector;

    #[async_trait::async_trait]
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

    let sandbox = SaveWorkspace::new("tool-overwrite");
    sandbox.seed("chart.html", b"original bytes");
    let tools =
        crate::agent::tools::DatabaseTools::new(Some(Box::new(NonEmptyConnector)), 100, true)
            .with_workspace(Some(Arc::new(sandbox.workspace.clone())));
    let error = tools
        .execute(
            "render_chart",
            serde_json::json!({
                "sql": "SELECT 1",
                "chart_type": "bar",
                "save_to": "chart.html"
            }),
        )
        .await
        .expect_err("an overwrite without a digest must refuse");
    match error {
        ToolError::WorkspaceWrite(message) => {
            assert!(
                message.contains("expected_digest") && message.contains("workspace_read"),
                "the refusal must carry the file tool's existing semantics: {message}"
            );
        }
        other => panic!("expected a typed workspace-write error, got: {other}"),
    }
    assert_eq!(
        fs::read(sandbox.root.join("chart.html")).expect("file must survive"),
        b"original bytes",
        "the pre-existing file must be byte-identical after the refused save"
    );
}
