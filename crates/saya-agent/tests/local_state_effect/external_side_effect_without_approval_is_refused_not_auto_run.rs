/// A tool that declares `external_side_effect` *without* also
/// declaring `requires_approval` is a misconfiguration the policy refuses to
/// auto-run, rather than trusting the author to set both. The refusal surfaces
/// as a `ToolDenied` event with a reason naming the side effect, and the tool
/// never executes. This is the test that would catch a future tool setting
/// only one of the two flags.
#[tokio::test]
async fn external_side_effect_without_approval_is_refused_not_auto_run() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let provider = OneCallProvider {
        call: ToolCall {
            id: "c1".into(),
            name: "open_browser".into(),
            arguments: serde_json::json!({}),
        },
        turn: Mutex::new(0),
    };
    let sink = RecordingSink {
        events: events.clone(),
    };
    let external_only = ToolDefinition {
        name: "open_browser".into(),
        description: "opens something outside the agent".into(),
        read_only: true,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: true,
            requires_approval: false,
            local_state: LocalStateEffect::None,
        },
        concurrency: saya_agent::ToolConcurrency::Serial,
        completion: None,
    };
    let _ = run_agent_with_sink(
        &provider,
        &RecordingExecutor {
            calls: calls.clone(),
        },
        request(),
        vec![external_only],
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &sink,
        saya_agent::CancellationToken::new(),
    )
    .await
    .expect("a denial is not a turn-ending error");
    assert!(
        calls.lock().unwrap().is_empty(),
        "the misconfigured tool must not execute"
    );
    let reason = events
        .lock()
        .unwrap()
        .iter()
        .find_map(|event| match event {
            AgentEvent::ToolDenied { name, reason } if name == "open_browser" => {
                Some(reason.clone())
            }
            _ => None,
        })
        .expect("a ToolDenied event must be emitted");
    assert!(
        reason.contains("side effect"),
        "the reason must name the external side effect, got: {reason}"
    );
}
