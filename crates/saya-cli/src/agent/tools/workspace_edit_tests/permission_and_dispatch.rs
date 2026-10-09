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
        variants.iter().any(|variant| {
            variant["required"] == serde_json::json!(["old_text", "new_text", "expected_digest"])
        }),
        "the replace variant must require the observed digest"
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
                "expected_digest": observed_digest(&sandbox, "notes.md"),
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
        call: edit_call(&sandbox, "notes.md", "anchor", "ANCHOR"),
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
        call: edit_call(&sandbox, "notes.md", "anchor", "ANCHOR"),
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
                "expected_digest": "stale-placeholder-digest-no-workspace",
            }),
        )
        .await
        .expect_err("no workspace, no edit");
    assert_eq!(error, saya_agent::ToolError::WorkspaceUnavailable);
}
