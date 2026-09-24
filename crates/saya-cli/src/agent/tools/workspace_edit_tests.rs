//! Tests for the `workspace_edit` tool (`replace` variant only): the
//! model-facing anchored edit that resolves `old_text` to exactly one byte
//! range and commits through `Workspace::patch_range`. Every row of the
//! failure contract (§4) is a named property here; every refusal leaves the
//! file byte-identical.

use std::{fs, path::PathBuf, sync::Arc};

use super::*;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentLimits, AgentRequest, LocalStateEffect, ToolCall,
    ToolDefinition, ToolExecutor,
};
use saya_harness::workspace::Workspace;

use super::database_tools::WORKSPACE_EDIT_MAX_BYTES;

/// A sandbox workspace under the OS temp dir, removed on drop. The root sits
/// one level down (`outer/ws`) so a `..` escape has a real target to name.
struct Sandbox {
    outer: PathBuf,
    ws: Workspace,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let outer =
            std::env::temp_dir().join(format!("saya-wsedit-{label}-{}", std::process::id()));
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
fn workspace_edit_definition() -> ToolDefinition {
    DatabaseTools::definitions(false, false, false, true, false)
        .into_iter()
        .find(|tool| tool.name == "workspace_edit")
        .expect("workspace_edit is advertised when workspace writes are permitted")
}

fn edit_call(path: &str, old_text: &str, new_text: &str) -> ToolCall {
    ToolCall {
        id: "c1".into(),
        name: "workspace_edit".into(),
        arguments: serde_json::json!({"path": path, "old_text": old_text, "new_text": new_text}),
    }
}

/// A provider that emits one tool call on its first `stream` invocation and a
/// plain text answer afterwards, so the run completes instead of looping on
/// the turn limit. Mirrors the harness in `workspace_write_tests`.
struct OneCallProvider {
    call: ToolCall,
    turn: std::sync::Mutex<u32>,
}

#[async_trait::async_trait]
impl saya_agent::ChatProvider for OneCallProvider {
    fn name(&self) -> &str {
        "one-call-mock"
    }
    async fn complete(
        &self,
        _: saya_agent::ChatRequest,
    ) -> Result<saya_agent::ChatResponse, saya_agent::ProviderError> {
        unreachable!("stream path is used")
    }
    async fn stream(
        &self,
        _: saya_agent::ChatRequest,
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
        prompt: "edit a file".into(),
        profile_names: Vec::new(),
        model: "mock-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

/// Zero matches: the tool refuses, names the count, and writes nothing.
#[tokio::test]
async fn zero_matches_writes_nothing() {
    let sandbox = Sandbox::new("zero");
    sandbox
        .ws
        .write("notes.md", b"hello workspace\n")
        .expect("seed write must succeed");
    let before = fs::read(sandbox.ws_root().join("notes.md")).expect("seed file must exist");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "no such anchor anywhere",
                "new_text": "replacement",
            }),
        )
        .await
        .expect_err("zero matches must refuse, not write");
    let text = error.to_string();
    assert!(
        text.contains("matched 0"),
        "the refusal must name the count, got: {text}"
    );
    assert!(
        text.contains("size:") && text.contains("digest:"),
        "the refusal names the current size and digest so the model can re-anchor: {text}"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must survive"),
        before,
        "a refused edit leaves the file byte-identical"
    );
}

/// Multiple matches: the tool refuses with bounded line numbers — never
/// "first wins" — and writes nothing.
#[tokio::test]
async fn multiple_matches_writes_nothing_and_reports_the_lines() {
    let sandbox = Sandbox::new("multi");
    let seed = b"line one: needle here\nline two: filler\nline three: needle here\n";
    sandbox
        .ws
        .write("notes.md", seed)
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "needle here",
                "new_text": "replacement",
            }),
        )
        .await
        .expect_err("multiple matches must refuse, not pick the first");
    match error {
        saya_agent::ToolError::WorkspaceEditAmbiguous { matches, lines, .. } => {
            assert_eq!(matches, 2, "the refusal must name the count");
            assert_eq!(
                lines,
                vec![1, 3],
                "the refusal reports bounded line numbers"
            );
        }
        other => panic!("expected a typed ambiguity refusal, got: {other}"),
    }
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must survive"),
        seed,
        "a refused edit leaves the file byte-identical"
    );
}

/// A moved anchor: an `expected_size`/`expected_digest` mismatch against the
/// file's current state refuses with no write.
#[tokio::test]
async fn a_moved_anchor_refuses_on_the_precondition() {
    let sandbox = Sandbox::new("moved");
    sandbox
        .ws
        .write("notes.md", b"version one: anchor\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    // The model measured a stale size; the file no longer has it.
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "anchor",
                "new_text": "replacement",
                "expected_size": 9999,
            }),
        )
        .await
        .expect_err("a stale size precondition must refuse");
    let text = error.to_string();
    assert!(
        text.contains("changed since measured"),
        "the refusal must name the moved anchor: {text}"
    );
    // The model measured a stale digest; same refusal, no write.
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "anchor",
                "new_text": "replacement",
                "expected_digest": "deadbeef",
            }),
        )
        .await
        .expect_err("a stale digest precondition must refuse");
    let text = error.to_string();
    assert!(
        text.contains("changed since measured"),
        "the refusal must name the moved anchor: {text}"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must survive"),
        b"version one: anchor\n",
        "a refused edit leaves the file byte-identical"
    );
}

/// An empty anchor matches everywhere, so it is a validation error — never
/// an edit, never a write.
#[tokio::test]
async fn an_empty_anchor_is_rejected() {
    let sandbox = Sandbox::new("empty");
    sandbox
        .ws
        .write("notes.md", b"hello workspace\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "",
                "new_text": "replacement",
            }),
        )
        .await
        .expect_err("an empty anchor must be rejected");
    assert_eq!(
        error,
        saya_agent::ToolError::WorkspaceEditEmptyAnchor {
            path: "notes.md".into(),
        }
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must survive"),
        b"hello workspace\n",
        "a refused edit leaves the file byte-identical"
    );
}

/// A replacement over the bound refuses whole — never truncated — and leaves
/// no partial edit behind.
#[tokio::test]
async fn a_replacement_over_the_bound_refuses_whole() {
    let sandbox = Sandbox::new("bound");
    sandbox
        .ws
        .write("notes.md", b"hello anchor world\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let oversized = "a".repeat(WORKSPACE_EDIT_MAX_BYTES + 1);
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "anchor",
                "new_text": oversized,
            }),
        )
        .await
        .expect_err("over the bound must be refused whole");
    match error {
        saya_agent::ToolError::WorkspaceEditTooLarge { limit, found, .. } => {
            assert_eq!(limit, WORKSPACE_EDIT_MAX_BYTES);
            assert_eq!(found, WORKSPACE_EDIT_MAX_BYTES + 1);
        }
        other => panic!("expected a typed over-bound refusal, got: {other}"),
    }
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must survive"),
        b"hello anchor world\n",
        "a refused edit leaves the file byte-identical"
    );
    // The boundary case: exactly the bound is a legal replacement.
    let exact = "a".repeat(WORKSPACE_EDIT_MAX_BYTES);
    let result = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "anchor",
                "new_text": exact,
            }),
        )
        .await
        .expect("a replacement exactly at the bound must be accepted");
    assert_eq!(result["path"], "notes.md");
}

/// An over-bound anchor refuses whole too: the range is never resolved, the
/// file never opened for a splice.
#[tokio::test]
async fn an_anchor_over_the_bound_refuses_whole() {
    let sandbox = Sandbox::new("anchor-bound");
    sandbox
        .ws
        .write("notes.md", b"hello workspace\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let oversized = "a".repeat(WORKSPACE_EDIT_MAX_BYTES + 1);
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": oversized,
                "new_text": "replacement",
            }),
        )
        .await
        .expect_err("an over-bound anchor must be refused whole");
    assert!(
        matches!(error, saya_agent::ToolError::WorkspaceEditTooLarge { .. }),
        "expected a typed over-bound refusal, got: {error}"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must survive"),
        b"hello workspace\n",
        "a refused edit leaves the file byte-identical"
    );
}

/// Errors carry counts, line numbers, sizes and digests — never unbounded
/// file content. A secret-shaped sentinel planted in the file must never
/// appear in any error payload.
#[tokio::test]
async fn an_error_payload_never_carries_file_content() {
    let sandbox = Sandbox::new("sentinel");
    // The sentinel is secret-shaped (`token=...`) so it would trip the
    // transcript redaction if it ever reached an error path; the contract is
    // stronger: it never reaches the payload at all.
    let sentinel = "token=SENTINEL-7f3a9c-must-never-leak";
    let seed = format!("header line\n{sentinel}\nfooter line\n");
    sandbox
        .ws
        .write("notes.md", seed.as_bytes())
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    // Zero matches: the refusal names the anchor argument (model-supplied),
    // never the file.
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "no such anchor",
                "new_text": "replacement",
            }),
        )
        .await
        .expect_err("zero matches must refuse");
    let text = error.to_string();
    assert!(
        !text.contains("SENTINEL-7f3a9c"),
        "a zero-match refusal must not carry file content: {text}"
    );
    // Multiple matches on a *different* anchor: the refusal names counts and
    // lines, never the sentinel line's content.
    sandbox
        .ws
        .write("dup.md", b"dup one\nfiller\ndup one\n")
        .expect("seed write must succeed");
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "dup.md",
                "old_text": "dup one",
                "new_text": "replacement",
            }),
        )
        .await
        .expect_err("multiple matches must refuse");
    let text = error.to_string();
    assert!(
        !text.contains("dup one"),
        "an ambiguity refusal reports lines, not excerpts: {text}"
    );
    // The sentinel file itself is untouched.
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must survive"),
        seed.as_bytes(),
        "a refused edit leaves the file byte-identical"
    );
}

/// The definition is hidden unless workspace writes are permitted: the loop's
/// gate keys on the declared `WriteWorkspace` effect, and a write tool the
/// model can see but never use wastes context and invites retries.
#[test]
fn the_tool_is_hidden_when_writes_are_not_permitted() {
    let hidden = DatabaseTools::definitions(false, false, false, false, false);
    assert!(
        !hidden.iter().any(|tool| tool.name == "workspace_edit"),
        "workspace_edit must be hidden when writes are not permitted"
    );
    let shown = DatabaseTools::definitions(false, false, false, true, false);
    let tool = shown
        .iter()
        .find(|tool| tool.name == "workspace_edit")
        .expect("workspace_edit must be advertised when writes are permitted");
    assert!(
        !tool.read_only,
        "workspace_edit writes a file; it is not read-only"
    );
    assert!(!tool.effect.database_data);
    assert!(!tool.effect.external_side_effect);
    assert!(
        !tool.effect.requires_approval,
        "the scope approval and the permit are the gate; no per-call prompt (D7)"
    );
    assert_eq!(tool.effect.local_state, LocalStateEffect::WriteWorkspace);
    assert_eq!(tool.parameters["required"], serde_json::json!(["path"]));
    let variants = tool.parameters["oneOf"]
        .as_array()
        .expect("workspace_edit schema must expose its variants with oneOf");
    assert_eq!(variants.len(), 2);
    assert!(
        variants
            .iter()
            .any(|variant| { variant["required"] == serde_json::json!(["old_text", "new_text"]) })
    );
    assert!(
        variants
            .iter()
            .any(|variant| { variant["required"] == serde_json::json!(["offset", "chunk"]) })
    );
    assert_eq!(tool.completion.as_deref(), Some("workspace file edited"));
}

/// Happy path: a successful edit changes only the matched range — the bytes
/// before and after the anchor are untouched — and reports the new size and
/// digest.
#[tokio::test]
async fn a_successful_edit_changes_only_the_matched_range() {
    let sandbox = Sandbox::new("happy");
    sandbox
        .ws
        .write("notes.md", b"hello anchor world\nsecond line\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let result = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "anchor",
                "new_text": "ANCHOR",
            }),
        )
        .await
        .expect("a unique anchor must edit");
    assert_eq!(result["path"], "notes.md");
    assert_eq!(result["bytes_replaced"], 6);
    assert_eq!(result["bytes_written"], 6);
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must exist"),
        b"hello ANCHOR world\nsecond line\n",
        "only the matched range changes; the bytes around it are untouched"
    );
    // The reported size and digest describe the file after the edit.
    let after = fs::read(sandbox.ws_root().join("notes.md")).expect("file must exist");
    assert_eq!(result["size"], after.len() as u64);
    let digest = result["digest"].as_str().expect("digest must be a string");
    assert_eq!(digest.len(), 64, "the digest is a sha256 hex string");
}

/// With `permit_workspace_writes: false` (the default), the loop denies the
/// call before the executor runs — asserted on the filesystem, not just the
/// event stream. The loop's effect-keyed gate (`workspace_write_denied`,
/// which keys on `WriteWorkspace`, not on the tool name) covers
/// `workspace_edit` with no loop change.
#[tokio::test]
async fn workspace_edit_is_denied_by_the_loop_by_default_and_writes_nothing() {
    let sandbox = Sandbox::new("loop-deny");
    sandbox
        .ws
        .write("notes.md", b"hello anchor world\n")
        .expect("seed write must succeed");
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let provider = OneCallProvider {
        call: edit_call("notes.md", "anchor", "ANCHOR"),
        turn: std::sync::Mutex::new(0),
    };
    let token = saya_agent::CancellationToken::new();
    let output = saya_agent::run_agent_with_sink(
        &provider,
        &sandbox.tools(),
        request(),
        vec![workspace_edit_definition()],
        AgentLimits::default(),
        &saya_agent::AllowReadOnlyApproval,
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
    assert_eq!(name, "workspace_edit");
    assert!(
        reason.contains("workspace writes are not permitted"),
        "the denial must name the workspace gate, not approval: {reason}"
    );
    assert_eq!(output.tool_metadata[0].status, "denied");
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must survive"),
        b"hello anchor world\n",
        "no byte may change on a denied call"
    );
}

/// With the permit granted the loop runs the tool, and the file lands with
/// exactly the spliced bytes — end to end through the real executor.
#[tokio::test]
async fn workspace_edit_runs_in_the_loop_when_writes_are_permitted() {
    let sandbox = Sandbox::new("loop-allow");
    sandbox
        .ws
        .write("notes.md", b"hello anchor world\n")
        .expect("seed write must succeed");
    let provider = OneCallProvider {
        call: edit_call("notes.md", "anchor", "ANCHOR"),
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
        vec![workspace_edit_definition()],
        limits,
        &saya_agent::AllowReadOnlyApproval,
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
        fs::read(sandbox.ws_root().join("notes.md")).expect("the file must exist"),
        b"hello ANCHOR world\n",
        "the file must hold exactly the spliced bytes"
    );
}

/// No workspace attached (the state before a run engine opens one): the tool
/// denies with a typed error instead of pretending to edit.
#[tokio::test]
async fn workspace_edit_denies_when_no_workspace_is_attached() {
    let tools = DatabaseTools::with_registry(
        crate::connection::ConnectionRegistry::new("primary"),
        100,
        true,
        None,
    );
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "anchor",
                "new_text": "replacement",
            }),
        )
        .await
        .expect_err("no workspace, no edit");
    assert_eq!(error, saya_agent::ToolError::WorkspaceUnavailable);
}

/// Unknown arguments are rejected before anything else runs.
#[tokio::test]
async fn workspace_edit_rejects_unknown_arguments() {
    let sandbox = Sandbox::new("unknown-args");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "a",
                "new_text": "b",
                "sql": "SELECT 1",
            }),
        )
        .await
        .expect_err("unknown arguments must be rejected");
    assert_eq!(error, saya_agent::ToolError::UnsupportedProperty);
}

/// A non-string `old_text` is a typed error of its own, distinct from the
/// path's, so the model can fix the right argument. Same for `new_text` and
/// the optional `expected_*` precondition.
#[tokio::test]
async fn workspace_edit_rejects_mistyped_arguments() {
    let sandbox = Sandbox::new("arg-types");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({"path": "notes.md", "old_text": 7, "new_text": "b"}),
        )
        .await
        .expect_err("a non-string old_text must be rejected at validation");
    assert_eq!(error, saya_agent::ToolError::OldTextNotString);
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({"path": "notes.md", "old_text": "a", "new_text": 7}),
        )
        .await
        .expect_err("a non-string new_text must be rejected at validation");
    assert_eq!(error, saya_agent::ToolError::NewTextNotString);
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "a",
                "new_text": "b",
                "expected_size": "huge",
            }),
        )
        .await
        .expect_err("a non-integer expected_size must be rejected at validation");
    assert_eq!(error, saya_agent::ToolError::ExpectedSizeNotUint);
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "a",
                "new_text": "b",
                "expected_digest": 7,
            }),
        )
        .await
        .expect_err("a non-string expected_digest must be rejected at validation");
    assert_eq!(error, saya_agent::ToolError::ExpectedDigestNotString);
}

/// A non-UTF-8 target is refused loudly: reads are lossy, so a byte-exact
/// anchor cannot be trusted. Binary support is a later slice, not this one.
#[tokio::test]
async fn workspace_edit_refuses_a_non_utf8_target() {
    let sandbox = Sandbox::new("non-utf8");
    sandbox
        .ws
        .write("blob.bin", &[0x66, 0x6f, 0xff, 0xfe, 0x6f])
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "blob.bin",
                "old_text": "fo",
                "new_text": "bar",
            }),
        )
        .await
        .expect_err("a non-UTF-8 target must be refused");
    assert_eq!(
        error,
        saya_agent::ToolError::WorkspaceEditNotText {
            path: "blob.bin".into(),
        }
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("blob.bin")).expect("file must survive"),
        vec![0x66, 0x6f, 0xff, 0xfe, 0x6f],
        "a refused edit leaves the file byte-identical"
    );
}

/// A `..` escape is refused with the harness's containment reason, and the
/// sentinel planted outside the workspace is byte-for-byte unchanged.
#[tokio::test]
async fn workspace_edit_refuses_a_dot_dot_escape() {
    let sandbox = Sandbox::new("escape");
    fs::write(sandbox.outer.join("outside.txt"), b"sentinel").expect("sentinel plant must succeed");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "../outside.txt",
                "old_text": "sentinel",
                "new_text": "overwritten",
            }),
        )
        .await
        .expect_err("a path outside the workspace must be refused, not edited");
    let text = error.to_string();
    assert!(
        text.contains("escapes the run workspace"),
        "the refusal must name the containment failure: {text}"
    );
    assert_eq!(
        fs::read(sandbox.outer.join("outside.txt")).expect("sentinel must survive"),
        b"sentinel",
        "nothing may be written outside the workspace"
    );
}

// -- the redaction-placeholder guard: refuse an edit that would copy a -----
// -- masked tool-result marker back over the real value on disk. -----------

/// An edit that would replace the real value with the literal `[redacted]`
/// marker is refused — the same guard `workspace_write` applies — and the
/// file on disk keeps the real value.
#[tokio::test]
async fn workspace_edit_refuses_to_add_redaction_placeholders() {
    let sandbox = Sandbox::new("redaction-guard");
    let seed = b"a = f(token=real_value)\n";
    sandbox
        .ws
        .write("a.py", seed)
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "a.py",
                "old_text": "token=real_value",
                "new_text": "token=[redacted]",
            }),
        )
        .await
        .expect_err("an edit that adds a redaction placeholder must be refused");
    let text = error.to_string();
    assert!(
        text.contains("[redacted]"),
        "the refusal must name the marker: {text}"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("a.py")).expect("file must survive"),
        seed,
        "a refused edit leaves the real value on disk untouched"
    );
}

/// A file that already holds the marker (e.g. an earlier legitimate write)
/// can still be edited as long as the edit does not grow the count: the
/// guard is a growth check, not a ban on the literal text.
#[tokio::test]
async fn a_file_that_already_holds_the_marker_can_still_be_edited() {
    let sandbox = Sandbox::new("marker-stays-flat");
    let seed = b"status: [redacted]\nother: line\n";
    sandbox
        .ws
        .write("notes.md", seed)
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "other: line",
                "new_text": "other: replaced",
            }),
        )
        .await
        .expect("the count stays at 1, so the edit must be allowed");
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must exist"),
        b"status: [redacted]\nother: replaced\n".to_vec(),
        "the edit must land; the pre-existing marker is untouched"
    );
}
