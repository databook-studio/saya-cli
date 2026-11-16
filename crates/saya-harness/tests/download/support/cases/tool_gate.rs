use super::*;

/// The default runner denies `http_download` (its declaration:
/// `external_side_effect` + `WriteWorkspace`), the tool never executes, and
/// no file appears in the workspace.
#[tokio::test]
async fn the_default_denies_http_download_and_no_file_appears() {
    let content = b"payload".to_vec();
    let (root, workspace) = workspace("deny");
    let site = LocalSite::spawn(vec![("/doc", route(content.as_slice(), 4))]).await;
    let net = transport(&site);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let executor = DownloadExecutor {
        policy: policy(),
        net,
        workspace,
        calls: calls.clone(),
    };
    let sink = RecordingSink {
        events: events.clone(),
    };
    let output = run_agent_with_sink(
        &OneCallProvider {
            call: ToolCall {
                id: "c1".into(),
                name: "http_download".into(),
                arguments: serde_json::json!({"url": url("/doc"), "destination": DEST}),
            },
            turn: Mutex::new(0),
        },
        &executor,
        agent_request(),
        vec![http_download_definition()],
        AgentLimits::default(),
        &saya_agent::AllowReadOnlyApproval,
        &sink,
        CancellationToken::new(),
    )
    .await
    .expect("a denial is not a turn-ending error");
    assert!(
        calls.lock().unwrap().is_empty(),
        "the tool must not execute when workspace writes are not permitted"
    );
    let reason = events
        .lock()
        .unwrap()
        .iter()
        .find_map(|event| match event {
            AgentEvent::ToolDenied { name, reason } if name == "http_download" => {
                Some(reason.clone())
            }
            _ => None,
        })
        .expect("a ToolDenied event must be emitted");
    assert!(!reason.is_empty(), "the denial names its gate: {reason}");
    assert!(
        std::fs::read_dir(&root)
            .expect("workspace")
            .next()
            .is_none(),
        "no file appears: the workspace is untouched"
    );
    assert_eq!(output.tool_metadata[0].status, "denied");
}

/// The definition's `WriteWorkspace` declaration feeds the workspace permit
/// gate specifically: with approval granted (a variant that requires it, so
/// the side-effect gate does not bind), the default runner still refuses and
/// the denial names the workspace gate.
#[tokio::test]
async fn the_workspace_permit_gate_is_what_refuses_the_download_tool() {
    let content = b"payload".to_vec();
    let (root, workspace) = workspace("gate");
    let site = LocalSite::spawn(vec![("/doc", route(content.as_slice(), 4))]).await;
    let net = transport(&site);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let executor = DownloadExecutor {
        policy: policy(),
        net,
        workspace,
        calls: calls.clone(),
    };
    let sink = RecordingSink {
        events: events.clone(),
    };
    let mut requires_approval = http_download_definition();
    requires_approval.effect.requires_approval = true;
    let output = run_agent_with_sink(
        &OneCallProvider {
            call: ToolCall {
                id: "c1".into(),
                name: "http_download".into(),
                arguments: serde_json::json!({"url": url("/doc"), "destination": DEST}),
            },
            turn: Mutex::new(0),
        },
        &executor,
        agent_request(),
        vec![requires_approval],
        AgentLimits::default(),
        &AllowApproval,
        &sink,
        CancellationToken::new(),
    )
    .await
    .expect("a denial is not a turn-ending error");
    assert!(
        calls.lock().unwrap().is_empty(),
        "the tool must not execute"
    );
    let reason = events
        .lock()
        .unwrap()
        .iter()
        .find_map(|event| match event {
            AgentEvent::ToolDenied { name, reason } if name == "http_download" => {
                Some(reason.clone())
            }
            _ => None,
        })
        .expect("a ToolDenied event must be emitted");
    assert!(
        reason.contains("workspace"),
        "the workspace permit gate is the refusing one: {reason}"
    );
    assert!(
        std::fs::read_dir(&root)
            .expect("workspace")
            .next()
            .is_none(),
        "no file appears"
    );
    assert_eq!(output.tool_metadata[0].status, "denied");
}
