//! Tests for the chart save's guarded publication (`chart_save`): the publish
//! step runs against a real temp workspace, and the tool-level tests cover
//! only refusal outcomes — every refusal returns before the private temp copy
//! and the browser open, so no test here ever opens a browser.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use saya_agent::{ToolError, ToolExecutor};
use saya_harness::workspace::{MAX_IO_BYTES, Workspace};

use super::database_tools::chart_save::publish_chart_html;

#[tokio::test]
async fn run_step_refuses_chart_save_without_its_workspace_write_permit() {
    let database = crate::agent::tools::DatabaseTools::with_registry(
        crate::connection::ConnectionRegistry::new("primary"),
        100,
        true,
        None,
    );
    let tools = super::run_tools::RunTools::compose(Arc::new(database), None, None, None);
    let error = tools
        .execute(
            "render_chart",
            serde_json::json!({"sql": "SELECT 1", "chart_type": "bar", "save_to": "chart.html"}),
        )
        .await
        .expect_err("a step without workspace-write cannot save a chart");
    assert_eq!(
        error,
        ToolError::WorkspaceWrite("workspace-write is not permitted for this step".into())
    );
}

/// A sandbox workspace under the OS temp dir, removed on drop.
struct SaveWorkspace {
    root: PathBuf,
    workspace: Workspace,
}

impl SaveWorkspace {
    fn new(label: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("saya-chart-publish-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).expect("sandbox directory must create");
        let workspace = Workspace::open(&root).expect("workspace root must open");
        Self { root, workspace }
    }

    fn seed(&self, rel: &str, bytes: &[u8]) {
        self.workspace
            .create_new(rel, bytes)
            .expect("seed file must create");
    }

    fn observed_digest(&self, rel: &str) -> String {
        self.workspace
            .read(rel, MAX_IO_BYTES as u64)
            .expect("the file must read back")
            .digest
    }
}

impl Drop for SaveWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

// -- the publish step against a real workspace --------------------------------

/// A no-digest publish to an absent path creates the file — the create form.
#[test]
fn publish_creates_a_new_file_without_a_digest() {
    let sandbox = SaveWorkspace::new("create");
    publish_chart_html(&sandbox.workspace, "chart.html", b"<html></html>", None)
        .expect("a no-digest publish to an absent path must create the file");
    assert_eq!(
        fs::read(sandbox.root.join("chart.html")).expect("file must exist"),
        b"<html></html>"
    );
}

/// The defining refusal: publishing over an existing file without a digest
/// changes nothing and surfaces the same text `workspace_write` surfaces.
#[test]
fn publish_refuses_over_an_existing_file_without_a_digest() {
    let sandbox = SaveWorkspace::new("no-digest");
    sandbox.seed("chart.html", b"original bytes");
    let error = publish_chart_html(&sandbox.workspace, "chart.html", b"<html></html>", None)
        .expect_err("replacing an existing file without a digest must refuse");
    match error {
        ToolError::WorkspaceWrite(message) => {
            assert!(
                message.contains("expected_digest") && message.contains("workspace_read"),
                "the refusal must tell the model how to replace: {message}"
            );
        }
        other => panic!("expected a typed workspace-write error, got: {other}"),
    }
    assert_eq!(
        fs::read(sandbox.root.join("chart.html")).expect("file must survive"),
        b"original bytes",
        "the file must be byte-identical after the refused publish"
    );
}

/// A stale digest refuses: the file moved since it was observed, and the
/// other writer's bytes survive — never last-writer-wins.
#[test]
fn publish_refuses_a_stale_digest_and_preserves_the_bytes() {
    let sandbox = SaveWorkspace::new("stale");
    sandbox.seed("chart.html", b"original bytes");
    let observed = sandbox.observed_digest("chart.html");
    sandbox
        .workspace
        .write("chart.html", b"edited bytes")
        .expect("the external overwrite must land");
    let error = publish_chart_html(
        &sandbox.workspace,
        "chart.html",
        b"<html></html>",
        Some(&observed),
    )
    .expect_err("a stale digest must refuse the replace");
    match error {
        ToolError::WorkspaceWrite(message) => {
            assert!(
                message.contains("changed") && message.contains("workspace_read"),
                "the refusal must send the model back to a fresh read: {message}"
            );
        }
        other => panic!("expected a typed workspace-write error, got: {other}"),
    }
    assert_eq!(
        fs::read(sandbox.root.join("chart.html")).expect("file must survive"),
        b"edited bytes",
        "the other writer's bytes must survive a refused stale publish"
    );
}

/// With the digest the model observed, an unchanged file is replaced whole.
#[test]
fn publish_replaces_an_existing_file_with_the_observed_digest() {
    let sandbox = SaveWorkspace::new("replace");
    sandbox.seed("chart.html", b"original bytes");
    let observed = sandbox.observed_digest("chart.html");
    publish_chart_html(
        &sandbox.workspace,
        "chart.html",
        b"<html></html>",
        Some(&observed),
    )
    .expect("a digest-guarded replace of an unchanged file must succeed");
    assert_eq!(
        fs::read(sandbox.root.join("chart.html")).expect("file must exist"),
        b"<html></html>"
    );
}

/// A digest-carrying publish to an absent path is refused, never a create,
/// and creates nothing — not even the parent directory.
#[test]
fn publish_refuses_a_digest_for_a_missing_file() {
    let sandbox = SaveWorkspace::new("digest-missing");
    let error = publish_chart_html(
        &sandbox.workspace,
        "notes/chart.html",
        b"<html></html>",
        Some(&"0".repeat(64)),
    )
    .expect_err("a digest-carrying publish to a missing file must refuse");
    match error {
        ToolError::WorkspaceWrite(message) => {
            assert!(
                message.contains("does not exist") && message.contains("expected_digest"),
                "the refusal must point at the digest-vs-create confusion: {message}"
            );
        }
        other => panic!("expected a typed workspace-write error, got: {other}"),
    }
    assert!(
        !sandbox.root.join("notes").exists(),
        "a refused digest publish must create nothing, not even the parent"
    );
}

/// Chart HTML over the whole-file I/O bound refuses before any filesystem
/// contact — the chart's own bound, not the 64 KiB model-write bound.
#[test]
fn publish_refuses_over_bound_html() {
    let sandbox = SaveWorkspace::new("bound");
    let oversized = vec![b'x'; MAX_IO_BYTES + 1];
    let error = publish_chart_html(&sandbox.workspace, "chart.html", &oversized, None)
        .expect_err("over the bound must be refused whole");
    match error {
        ToolError::WorkspaceWrite(message) => {
            assert!(
                message.contains("over the") && message.contains(&MAX_IO_BYTES.to_string()),
                "the refusal must carry the chart's own bound: {message}"
            );
        }
        other => panic!("expected a typed workspace-write error, got: {other}"),
    }
    assert!(
        !sandbox.root.join("chart.html").exists(),
        "a refused publish must create nothing"
    );
}

// -- argument typing -----------------------------------------------------------

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

// -- the advertised schema ------------------------------------------------------

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

// -- through the real tool (refusals only: they return before any open) ---------

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
