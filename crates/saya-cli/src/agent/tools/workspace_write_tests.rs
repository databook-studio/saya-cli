//! Tests for the `workspace_write` tool: the first model-facing tool that
//! writes anything. The defining invariants: the loop's fail-closed permit
//! gate denies the call before the executor is reached (asserted on the
//! filesystem, not just the error), a permitted call lands exactly the bytes
//! given atomically, and every containment refusal surfaces the harness's
//! own reason while leaving the world outside the workspace untouched.

use std::{fs, path::PathBuf, sync::Arc};

use super::*;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentLimits, AgentRequest, AllowReadOnlyApproval, ChatProvider,
    ChatRequest, ChatResponse, LocalStateEffect, ToolCall, ToolDefinition, ToolError, ToolExecutor,
};
use saya_harness::workspace::Workspace;

use super::database_tools::WORKSPACE_WRITE_MAX_BYTES;

/// A sandbox workspace under the OS temp dir, removed on drop. The root sits
/// one level down (`outer/ws`) so a `../` escape has a real target
/// (`outer/outside.txt`) to name, and a symlink has a real victim to point at.
struct Sandbox {
    outer: PathBuf,
    ws: Workspace,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let outer =
            std::env::temp_dir().join(format!("saya-wswrite-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&outer);
        fs::create_dir_all(outer.join("ws")).expect("sandbox directory must create");
        let ws = Workspace::open(&outer.join("ws")).expect("workspace root must open");
        Self { outer, ws }
    }

    fn ws_root(&self) -> PathBuf {
        self.outer.join("ws")
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

/// The real definition, taken from the advertised list: the loop tests must
/// exercise the gate against the definition the model would actually see.
fn workspace_write_definition() -> ToolDefinition {
    DatabaseTools::definitions(false, false, false, true)
        .into_iter()
        .find(|tool| tool.name == "workspace_write")
        .expect("workspace_write is advertised when workspace writes are permitted")
}

fn write_call(path: &str, content: &str) -> ToolCall {
    ToolCall {
        id: "c1".into(),
        name: "workspace_write".into(),
        arguments: serde_json::json!({"path": path, "content": content}),
    }
}

/// A provider that emits one tool call on its first `stream` invocation and a
/// plain text answer afterwards, so the run completes instead of looping on
/// the turn limit. Mirrors the harness in `saya-agent`'s local-state tests.
struct OneCallProvider {
    call: ToolCall,
    turn: std::sync::Mutex<u32>,
}

#[async_trait::async_trait]
impl ChatProvider for OneCallProvider {
    fn name(&self) -> &str {
        "one-call-mock"
    }
    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, saya_agent::ProviderError> {
        unreachable!("stream path is used")
    }
    async fn stream(
        &self,
        _: ChatRequest,
        _: saya_agent::CancellationToken,
    ) -> Result<saya_agent::ProviderStream, saya_agent::ProviderError> {
        let first = {
            let mut turn = self.turn.lock().unwrap();
            let was = *turn;
            *turn += 1;
            was == 0
        };
        let events = if first {
            vec![
                Ok(saya_agent::ProviderEvent::ToolCalls(vec![
                    self.call.clone(),
                ])),
                Ok(saya_agent::ProviderEvent::Done),
            ]
        } else {
            vec![
                Ok(saya_agent::ProviderEvent::TextDelta("done".into())),
                Ok(saya_agent::ProviderEvent::Done),
            ]
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

struct RecordingSink {
    events: std::sync::Arc<std::sync::Mutex<Vec<AgentEvent>>>,
}

#[async_trait::async_trait]
impl AgentEventSink for RecordingSink {
    async fn emit(&self, event: AgentEvent) {
        self.events.lock().unwrap().push(event);
    }
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "write a file".into(),
        profile_names: Vec::new(),
        model: "mock-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

/// The refusal that matters most: with `permit_workspace_writes: false` (the
/// default), the loop denies the call before the executor runs — asserted on
/// the filesystem, not just the event stream. If the gate were removed this
/// test goes red because the file would exist.
#[tokio::test]
async fn workspace_write_is_denied_by_the_loop_by_default_and_writes_nothing() {
    let sandbox = Sandbox::new("loop-deny");
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let provider = OneCallProvider {
        call: write_call("notes/out.md", "loop wrote me"),
        turn: std::sync::Mutex::new(0),
    };
    let token = saya_agent::CancellationToken::new();
    let output = saya_agent::run_agent_with_sink(
        &provider,
        &sandbox.tools(),
        request(),
        vec![workspace_write_definition()],
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &RecordingSink {
            events: events.clone(),
        },
        token,
    )
    .await
    .expect("denial is not a turn-ending error");
    let denied = events.lock().unwrap().iter().find_map(|event| match event {
        AgentEvent::ToolDenied { name, reason } => Some((name.clone(), reason.clone())),
        _ => None,
    });
    let (name, reason) = denied.expect("a ToolDenied event must be emitted");
    assert_eq!(name, "workspace_write");
    assert!(
        reason.contains("workspace writes are not permitted"),
        "the denial must name the workspace gate, not approval: {reason}"
    );
    assert_eq!(output.tool_metadata[0].status, "denied");
    // The filesystem, not just the event stream: nothing was created.
    assert!(
        !sandbox.ws_root().join("notes").exists(),
        "no directory may be created by a denied call"
    );
    assert!(
        fs::read_dir(sandbox.ws_root())
            .expect("workspace root must still exist")
            .count()
            == 0,
        "the workspace must be untouched after a denied call"
    );
}

/// With the permit granted the loop runs the tool, and the file lands with
/// exactly the bytes given — end to end through the real executor.
#[tokio::test]
async fn workspace_write_runs_in_the_loop_when_writes_are_permitted() {
    let sandbox = Sandbox::new("loop-allow");
    let provider = OneCallProvider {
        call: write_call("notes/out.md", "loop wrote me"),
        turn: std::sync::Mutex::new(0),
    };
    let token = saya_agent::CancellationToken::new();
    let limits = AgentLimits {
        permit_workspace_writes: true,
        ..AgentLimits::default()
    };
    let output = saya_agent::run_agent_with_sink(
        &provider,
        &sandbox.tools(),
        request(),
        vec![workspace_write_definition()],
        limits,
        &AllowReadOnlyApproval,
        &RecordingSink {
            events: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        },
        token,
    )
    .await
    .expect("run completes");
    assert!(
        !output
            .events
            .iter()
            .any(|event| matches!(event, AgentEvent::ToolDenied { .. })),
        "no denial when workspace writes are permitted"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes/out.md")).expect("the file must exist"),
        b"loop wrote me",
        "the file must hold exactly the bytes the call carried"
    );
}

/// The executor surface: a permitted write lands the exact bytes.
#[tokio::test]
async fn workspace_write_lands_exactly_the_bytes_given() {
    let sandbox = Sandbox::new("exact");
    let tools = sandbox.tools();
    let result = tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "notes/report.md", "content": "hello workspace"}),
        )
        .await
        .expect("a permitted contained write must succeed");
    assert_eq!(result["path"], "notes/report.md");
    assert_eq!(result["bytes_written"], 15);
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes/report.md")).expect("file must exist"),
        b"hello workspace"
    );
}

/// The harness's mode discipline holds through the tool: the file is 0600 and
/// carries no execute bit.
#[cfg(unix)]
#[tokio::test]
async fn workspace_write_produces_a_0600_file_without_execute_bits() {
    use std::os::unix::fs::PermissionsExt;

    let sandbox = Sandbox::new("mode");
    let tools = sandbox.tools();
    tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "script.sh", "content": "#!/bin/sh\necho hi\n"}),
        )
        .await
        .expect("a permitted contained write must succeed");
    let mode = fs::metadata(sandbox.ws_root().join("script.sh"))
        .expect("file must exist")
        .permissions()
        .mode();
    assert_eq!(
        mode & 0o777,
        0o600,
        "the file must be owner-read/write only"
    );
    assert_eq!(mode & 0o111, 0, "no execute bit may ever be set");
}

/// A `..` escape is refused with the harness's containment reason, and the
/// sentinel planted outside the workspace is byte-for-byte unchanged.
#[tokio::test]
async fn workspace_write_refuses_a_dot_dot_escape_and_leaves_the_outside_untouched() {
    let sandbox = Sandbox::new("escape");
    fs::write(sandbox.outer.join("outside.txt"), b"sentinel").expect("sentinel plant must succeed");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "../outside.txt", "content": "overwritten"}),
        )
        .await
        .expect_err("a path outside the workspace must be refused, not written");
    match error {
        ToolError::WorkspaceWrite(message) => {
            assert!(
                message.contains("escapes the run workspace"),
                "the refusal must name the containment failure: {message}"
            );
            assert!(
                message.contains("../outside.txt"),
                "the refusal must name the refused path: {message}"
            );
        }
        other => panic!("expected a typed workspace-write error, got: {other}"),
    }
    assert_eq!(
        fs::read(sandbox.outer.join("outside.txt")).expect("sentinel must survive"),
        b"sentinel",
        "nothing may be written outside the workspace"
    );
}

/// An absolute path is refused the same way as an escape — never resolved
/// against the filesystem.
#[tokio::test]
async fn workspace_write_refuses_an_absolute_path() {
    let sandbox = Sandbox::new("absolute");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "/tmp/saya-should-not-write", "content": "no"}),
        )
        .await
        .expect_err("an absolute path must be refused");
    match error {
        ToolError::WorkspaceWrite(message) => {
            assert!(
                message.contains("escapes the run workspace"),
                "the refusal must name the containment failure: {message}"
            );
        }
        other => panic!("expected a typed workspace-write error, got: {other}"),
    }
}

/// A symlinked path is refused, never followed: the victim outside the
/// workspace keeps its bytes and the link itself is not replaced by a file.
/// (Unix-only: the symlink plant needs the unix fs API.)
#[cfg(unix)]
#[tokio::test]
async fn workspace_write_refuses_a_symlinked_path() {
    let sandbox = Sandbox::new("symlink");
    fs::write(sandbox.outer.join("victim.txt"), b"victim").expect("victim plant must succeed");
    std::os::unix::fs::symlink(
        sandbox.outer.join("victim.txt"),
        sandbox.ws_root().join("link"),
    )
    .expect("symlink plant must succeed");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "link", "content": "overwritten?"}),
        )
        .await
        .expect_err("a symlinked path must be refused, not followed");
    match error {
        ToolError::WorkspaceWrite(message) => {
            assert!(
                message.contains("symlink"),
                "the refusal must name the symlink refusal: {message}"
            );
        }
        other => panic!("expected a typed workspace-write error, got: {other}"),
    }
    assert_eq!(
        fs::read(sandbox.outer.join("victim.txt")).expect("victim must survive"),
        b"victim",
        "the link must not have been followed"
    );
    assert!(
        sandbox
            .ws_root()
            .join("link")
            .symlink_metadata()
            .expect("link must survive")
            .file_type()
            .is_symlink(),
        "the link must not have been replaced by a regular file"
    );
}

/// Content over the byte bound is a typed refusal carrying the bound — never
/// a truncated write — and leaves no file behind. Exactly at the bound is
/// allowed, so the bound is a ceiling, not a hint.
#[tokio::test]
async fn workspace_write_refuses_content_over_the_bound_and_leaves_no_file() {
    let sandbox = Sandbox::new("bound");
    let tools = sandbox.tools();
    let oversized = "a".repeat(WORKSPACE_WRITE_MAX_BYTES + 1);
    let error = tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "big.txt", "content": oversized}),
        )
        .await
        .expect_err("over the bound must be refused whole");
    assert_eq!(
        error,
        ToolError::WorkspaceWriteTooLarge {
            limit: WORKSPACE_WRITE_MAX_BYTES
        }
    );
    assert!(
        !sandbox.ws_root().join("big.txt").exists(),
        "a refused write must leave no file behind"
    );
    // The boundary case: exactly the bound is a legal write.
    let exact = "a".repeat(WORKSPACE_WRITE_MAX_BYTES);
    tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "exact.txt", "content": exact}),
        )
        .await
        .expect("content exactly at the bound must be accepted");
    assert_eq!(
        fs::metadata(sandbox.ws_root().join("exact.txt"))
            .expect("file must exist")
            .len(),
        WORKSPACE_WRITE_MAX_BYTES as u64
    );
}

/// Writing over an existing file replaces it whole: the path never holds a
/// mix of old and new content, and no temp file survives the replace.
#[tokio::test]
async fn workspace_write_replaces_an_existing_file_whole() {
    let sandbox = Sandbox::new("replace");
    let tools = sandbox.tools();
    tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "report.md", "content": "0123456789ABCDEF"}),
        )
        .await
        .expect("the first write must succeed");
    tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "report.md", "content": "hi"}),
        )
        .await
        .expect("a permitted replace must succeed");
    assert_eq!(
        fs::read(sandbox.ws_root().join("report.md")).expect("file must exist"),
        b"hi",
        "the path must hold exactly the new content — no partial window's debris"
    );
    let leftovers: Vec<String> = fs::read_dir(sandbox.ws_root())
        .expect("root must exist")
        .map(|entry| {
            entry
                .expect("entries must read")
                .file_name()
                .into_string()
                .expect("utf-8")
        })
        .collect();
    assert_eq!(leftovers, vec!["report.md"], "no temp file may survive");
}

/// No workspace attached (the state before a run engine opens one): the tool
/// denies with a typed error instead of pretending to write.
#[tokio::test]
async fn workspace_write_denies_when_no_workspace_is_attached() {
    let tools = DatabaseTools::with_registry(
        crate::connection::ConnectionRegistry::new("primary"),
        100,
        true,
        None,
    );
    let error = tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "notes.md", "content": "hi"}),
        )
        .await
        .expect_err("no workspace, no write");
    assert_eq!(error, ToolError::WorkspaceUnavailable);
    assert!(
        error.to_string().contains("no workspace is available"),
        "the denial must be readable by the model: {error}"
    );
}

/// Unknown arguments are rejected before anything else runs.
#[tokio::test]
async fn workspace_write_rejects_unknown_arguments() {
    let sandbox = Sandbox::new("unknown-args");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "notes.md", "content": "hi", "sql": "SELECT 1"}),
        )
        .await
        .expect_err("unknown arguments must be rejected");
    assert_eq!(error, ToolError::UnsupportedProperty);
}

/// A non-string `content` is a typed error of its own, distinct from the
/// path's, so the model can fix the right argument.
#[tokio::test]
async fn workspace_write_rejects_a_non_string_content() {
    let sandbox = Sandbox::new("content-type");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "notes.md", "content": 7}),
        )
        .await
        .expect_err("a non-string content must be rejected at validation");
    assert_eq!(error, ToolError::ContentNotString);
}

/// The definition is hidden unless workspace writes are permitted: a tool the
/// model can see but never use wastes context and invites retries.
#[test]
fn workspace_write_definition_is_hidden_until_writes_are_permitted() {
    let hidden = DatabaseTools::definitions(false, false, false, false);
    assert!(
        !hidden.iter().any(|tool| tool.name == "workspace_write"),
        "workspace_write must be hidden when writes are not permitted"
    );
    let shown = DatabaseTools::definitions(false, false, false, true);
    assert!(
        shown.iter().any(|tool| tool.name == "workspace_write"),
        "workspace_write must be advertised when writes are permitted"
    );
}

/// The definition declares an honest effect: `WriteWorkspace`, no approval
/// (the permit is the gate, not a per-call prompt), its own completion — and
/// both arguments required.
#[test]
fn workspace_write_definition_declares_an_honest_effect() {
    let tools = DatabaseTools::definitions(false, false, false, true);
    let tool = tools
        .iter()
        .find(|tool| tool.name == "workspace_write")
        .expect("workspace_write is advertised when writes are permitted");
    assert!(
        !tool.read_only,
        "workspace_write writes a file; it is not read-only"
    );
    assert!(!tool.effect.database_data);
    assert!(!tool.effect.external_side_effect);
    assert!(
        !tool.effect.requires_approval,
        "the scope approval and the permit are the gate; no per-call prompt (D7)"
    );
    assert_eq!(tool.effect.local_state, LocalStateEffect::WriteWorkspace);
    assert_eq!(
        tool.parameters["required"],
        serde_json::json!(["path", "content"])
    );
    assert_eq!(tool.completion.as_deref(), Some("workspace file written"));
}
