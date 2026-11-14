//! Tests for the `read` tool: the one contained, bounded file read
//! the model's tools can perform. The defining invariants: a path outside the
//! workspace surfaces as a typed, model-readable `ToolError` (never a panic,
//! never a silent empty result), and the byte bound truncates with the
//! truncation reported in the result rather than silently dropping bytes.

use std::{fs, path::PathBuf, sync::Arc};

use super::*;
use saya_agent::{LocalStateEffect, ToolError, ToolExecutor};
use saya_harness::workspace::Workspace;

use super::database_tools::{MAX_IMAGE_BYTES, WORKSPACE_READ_MAX_BYTES};

#[path = "workspace_read_tests/image_read.rs"]
mod image_read;
#[path = "workspace_read_tests/notebook_images.rs"]
mod notebook_images;
#[path = "workspace_read_tests/notebook_read.rs"]
mod notebook_read;
#[path = "workspace_read_tests/numbered_lines.rs"]
mod numbered_lines;
#[path = "workspace_read_tests/read_bound.rs"]
mod read_bound;
#[path = "workspace_read_tests/worker_lifecycle.rs"]
mod worker_lifecycle;

/// A sandbox workspace under the OS temp dir, removed on drop. The root sits
/// one level down (`outer/ws`) so a `../` escape has a real target
/// (`outer/outside.txt`) to name.
struct Sandbox {
    outer: PathBuf,
    ws: Workspace,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let outer =
            std::env::temp_dir().join(format!("saya-wsread-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&outer);
        fs::create_dir_all(outer.join("ws")).expect("sandbox directory must create");
        let ws = Workspace::open(&outer.join("ws")).expect("workspace root must open");
        Self { outer, ws }
    }

    /// Database tools with the sandbox workspace attached and no connections —
    /// a workspace-only run has no selected profile.
    fn tools(&self) -> DatabaseTools {
        DatabaseTools::with_registry(
            crate::connection::ConnectionRegistry::new("primary"),
            100,
            true,
            None,
        )
        .with_workspace(Some(Arc::new(self.ws.clone())))
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.outer);
    }
}

/// The file's own bytes behind the numbered rendering: drop the six-column
/// number and `→` that begins each delivered line.
fn unnumbered(content: &str) -> String {
    content
        .split_inclusive('\n')
        .map(|line| line.split_once('→').map_or(line, |(_, text)| text))
        .collect()
}

/// The description rides every request of every run, so it stays short. The
/// image and notebook clauses are one sentence that still names both shapes,
/// and the whole text sits well inside its bound.
#[test]
fn read_advertises_the_lean_bounded_description() {
    let description = DatabaseTools::definitions(false, false, false, false)
        .into_iter()
        .find(|tool| tool.name == "read")
        .expect("read is advertised on every surface")
        .description;
    assert!(
        description.chars().count() <= 300,
        "read's description is lean: {} chars",
        description.chars().count()
    );
    for argument in ["path", "start_line", "line_count", "digest", "truncated"] {
        assert!(
            description.contains(argument),
            "read's description names `{argument}`: {description}"
        );
    }
}

#[tokio::test]
async fn workspace_read_returns_the_contained_content() {
    let sandbox = Sandbox::new("content");
    sandbox
        .ws
        .write("notes/summary.md", b"hello workspace")
        .expect("contained write must succeed");
    let tools = sandbox.tools();
    let result = tools
        .execute("read", serde_json::json!({"path": "notes/summary.md"}))
        .await
        .expect("a contained read must succeed");
    assert_eq!(result["path"], "notes/summary.md");
    assert_eq!(result["content"], "     1→hello workspace");
    assert_eq!(result["size"].as_u64(), Some(15));
    assert!(
        result.get("truncated").is_none(),
        "a whole-file read carries no `truncated`: {result}"
    );
}

/// An absolute path under the root is the same file as its relative
/// spelling: the result carries the relative form, so the read witness a
/// later write checks is keyed the same way.
#[tokio::test]
async fn workspace_read_accepts_an_absolute_path_under_the_root() {
    let sandbox = Sandbox::new("absolute");
    sandbox
        .ws
        .write("notes/summary.md", b"hello workspace")
        .expect("contained write must succeed");
    let tools = sandbox.tools();
    let absolute = sandbox.ws.root().join("notes/summary.md");
    let result = tools
        .execute(
            "read",
            serde_json::json!({"path": absolute.to_str().expect("utf-8 path")}),
        )
        .await
        .expect("an absolute path under the root must be served");
    assert_eq!(result["path"], "notes/summary.md");
    assert_eq!(result["content"], "     1→hello workspace");
}

/// An absolute path outside the root stays refused, and the refusal names
/// the root so the model can correct the path.
#[tokio::test]
async fn workspace_read_refuses_an_absolute_path_outside_the_root_naming_it() {
    let sandbox = Sandbox::new("absolute-outside");
    let outside = sandbox
        .ws
        .root()
        .parent()
        .expect("the sandbox root has a parent")
        .join("outside.txt");
    fs::write(&outside, b"secret").expect("plant must succeed");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "read",
            serde_json::json!({"path": outside.to_str().expect("utf-8 path")}),
        )
        .await
        .expect_err("an absolute path outside the root must be refused");
    match error {
        ToolError::Workspace(message) => {
            assert!(
                message.contains(&sandbox.ws.root().display().to_string()),
                "the refusal must name the root: {message}"
            );
            assert!(
                message.contains("outside.txt"),
                "the refusal must name the refused path: {message}"
            );
        }
        other => panic!("expected a typed workspace error, got: {other}"),
    }
}

/// The escape the containment layer exists for: a `..` path that rises above
/// the root is refused as a typed `ToolError` carrying the harness's own
/// reason — readable by the model, not a panic and not a silent empty result.
#[tokio::test]
async fn workspace_read_surfaces_a_path_outside_the_workspace_as_a_typed_error() {
    let sandbox = Sandbox::new("escape");
    fs::write(sandbox.outer.join("outside.txt"), b"secret").expect("plant must succeed");
    let tools = sandbox.tools();
    let error = tools
        .execute("read", serde_json::json!({"path": "../outside.txt"}))
        .await
        .expect_err("a path outside the workspace must be refused, not served");
    match error {
        ToolError::Workspace(message) => {
            assert!(
                message.contains("escapes the run workspace"),
                "the refusal must name the containment failure: {message}"
            );
            assert!(
                message.contains("../outside.txt"),
                "the refusal must name the refused path: {message}"
            );
        }
        other => panic!("expected a typed workspace error, got: {other}"),
    }
}

/// The byte bound is enforced and the truncation is reported: content is
/// capped by both the read bound and the model-facing result budget, `size`
/// still reports the file's full length, and `truncated` is true — never silent.
#[tokio::test]
async fn workspace_read_truncates_at_the_byte_bound_and_reports_it() {
    let sandbox = Sandbox::new("truncate");
    let total = WORKSPACE_READ_MAX_BYTES + 16;
    let source = vec![b'a'; total as usize];
    sandbox
        .ws
        .write("big.bin", &source)
        .expect("contained write must succeed");
    let tools = sandbox.tools();
    let result = tools
        .execute("read", serde_json::json!({"path": "big.bin"}))
        .await
        .expect("a bounded read must succeed");
    assert_eq!(result["truncated"], serde_json::Value::Bool(true));
    assert_eq!(result["size"].as_u64(), Some(total));
    let content = result["content"]
        .as_str()
        .expect("content must be a string");
    assert!(content.starts_with("     1→"));
    let text = unnumbered(content);
    assert!(!text.is_empty());
    assert!(text.len() as u64 <= WORKSPACE_READ_MAX_BYTES);
    assert!(source.starts_with(text.as_bytes()));
    assert!(text.bytes().all(|byte| byte == b'a'));
    let expected_digest = {
        use sha2::Digest;
        format!("{:x}", sha2::Sha256::digest(&source))
    };
    assert_eq!(result["digest"], expected_digest);
    let shaped = saya_agent::shape_tool_result(
        &result,
        saya_agent::AgentLimits::default().context_byte_budget,
    );
    assert!(!shaped.truncated, "the direct-call projection must fit");
}

/// No workspace attached (every current caller, until the run engine opens
/// one): the tool denies with a typed error instead of pretending to read.
#[tokio::test]
async fn workspace_read_denies_when_no_workspace_is_attached() {
    let tools = DatabaseTools::with_registry(
        crate::connection::ConnectionRegistry::new("primary"),
        100,
        true,
        None,
    );
    let error = tools
        .execute("read", serde_json::json!({"path": "notes.md"}))
        .await
        .expect_err("no workspace, no read");
    assert_eq!(error, ToolError::WorkspaceUnavailable);
    assert!(
        error.to_string().contains("no workspace is available"),
        "the denial must be readable by the model: {error}"
    );
}

/// The definition is advertised regardless of the data-sharing gate (it
/// touches no database data) and is read-shaped: read-only approval permits
/// it, and it declares `LocalStateEffect::Read`.
#[test]
fn workspace_read_definition_is_read_shaped_and_always_advertised() {
    let tools = DatabaseTools::definitions(false, false, false, false);
    let tool = tools
        .iter()
        .find(|tool| tool.name == "read")
        .expect("read is advertised even with the data gate closed");
    assert!(tool.read_only);
    assert!(!tool.effect.database_data);
    assert!(!tool.effect.external_side_effect);
    assert!(!tool.effect.requires_approval);
    assert_eq!(tool.effect.local_state, LocalStateEffect::Read);
    assert_eq!(tool.parameters["required"], serde_json::json!(["path"]));
    assert_eq!(tool.parameters["properties"]["start_line"]["minimum"], 1);
    assert_eq!(tool.parameters["properties"]["line_count"]["maximum"], 2000);
    assert_eq!(tool.completion.as_deref(), Some("workspace file read"));
}

#[tokio::test]
async fn workspace_read_rejects_a_non_string_path() {
    let tools = DatabaseTools::with_registry(
        crate::connection::ConnectionRegistry::new("primary"),
        100,
        true,
        None,
    );
    let error = tools
        .execute("read", serde_json::json!({"path": 7}))
        .await
        .expect_err("a non-string path must be rejected at validation");
    assert_eq!(error, ToolError::PathNotString);
}

#[tokio::test]
async fn workspace_read_rejects_unknown_arguments() {
    let tools = DatabaseTools::with_registry(
        crate::connection::ConnectionRegistry::new("primary"),
        100,
        true,
        None,
    );
    let error = tools
        .execute(
            "read",
            serde_json::json!({"path": "notes.md", "sql": "SELECT 1"}),
        )
        .await
        .expect_err("unknown arguments must be rejected");
    assert_eq!(error, ToolError::UnsupportedProperty);
}
