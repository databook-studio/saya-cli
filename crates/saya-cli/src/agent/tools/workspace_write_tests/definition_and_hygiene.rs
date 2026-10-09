/// The definition advertises `expected_digest` (optional, string-typed) and
/// still requires only `path` and `content` — omitting the digest is the
/// create form, so it must not be mandatory.
#[test]
fn workspace_write_definition_advertises_expected_digest() {
    let definition = workspace_write_definition();
    let properties = &definition.parameters["properties"];
    assert_eq!(properties["expected_digest"]["type"], "string");
    assert!(
        properties["expected_digest"]["description"]
            .as_str()
            .expect("expected_digest must carry a description")
            .contains("workspace_read"),
        "the argument's description must say where the digest comes from"
    );
    assert_eq!(
        definition.parameters["required"],
        serde_json::json!(["path", "content"]),
        "expected_digest stays optional: omitting it is the create form"
    );
    assert!(
        definition.description.contains("expected_digest"),
        "the tool description must state the digest precondition: {}",
        definition.description
    );
}

/// A non-string `expected_digest` is its own typed error, like the edit
/// tool's parser; an explicit `null` counts as absent, not mistyped.
#[tokio::test]
async fn workspace_write_rejects_a_non_string_expected_digest() {
    let sandbox = Sandbox::new("digest-type");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "notes.md", "content": "hi", "expected_digest": 7}),
        )
        .await
        .expect_err("a non-string digest must be rejected at validation");
    assert_eq!(error, ToolError::ExpectedDigestNotString);
    let result = tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "notes.md", "content": "hi", "expected_digest": null}),
        )
        .await
        .expect("an explicit null digest is the create form, not a mistyped one");
    assert_eq!(result["path"], "notes.md");
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

// -- the redaction-placeholder guard: refuse a write that would copy a ------
// -- masked tool-result marker back over the real value on disk. -----------

/// An existing file holds the real secret; writing back the content the
/// model would have seen after the D8 scrub (the value replaced with the
/// literal `[redacted]`) is refused, and the file on disk is byte-identical
/// afterwards — the real value must survive.
#[tokio::test]
async fn workspace_write_refuses_to_add_redaction_placeholders() {
    let sandbox = Sandbox::new("redaction-guard");
    let tools = sandbox.tools();
    tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "a.py", "content": "a = f(token=real_value)"}),
        )
        .await
        .expect("the seed write must succeed");
    let before = fs::read(sandbox.ws_root().join("a.py")).expect("seed file must exist");
    let error = tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "a.py", "content": "a = f(token=[redacted])"}),
        )
        .await
        .expect_err("a write that adds a redaction placeholder must be refused");
    let text = error.to_string();
    assert!(
        text.contains("[redacted]"),
        "the refusal must name the marker: {text}"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("a.py")).expect("file must survive"),
        before,
        "a refused write leaves the real value on disk untouched"
    );
}

/// A brand-new file (0 markers on disk, since it doesn't exist) holding the
/// literal marker is refused the same way — 0 to 1 is still growth.
#[tokio::test]
async fn a_new_file_with_the_marker_is_refused() {
    let sandbox = Sandbox::new("new-file-marker");
    let tools = sandbox.tools();
    tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "new.txt", "content": "leaked: [redacted]"}),
        )
        .await
        .expect_err("a new file holding the marker must be refused");
    assert!(
        !sandbox.ws_root().join("new.txt").exists(),
        "a refused write must leave no file behind"
    );
}

/// The definition is hidden unless workspace writes are permitted: a tool the
/// model can see but never use wastes context and invites retries.
#[test]
fn workspace_write_definition_is_hidden_until_writes_are_permitted() {
    let hidden = DatabaseTools::definitions(false, false, false, false, false);
    assert!(
        !hidden.iter().any(|tool| tool.name == "workspace_write"),
        "workspace_write must be hidden when writes are not permitted"
    );
    let shown = DatabaseTools::definitions(false, false, false, true, false);
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
    let tools = DatabaseTools::definitions(false, false, false, true, false);
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
