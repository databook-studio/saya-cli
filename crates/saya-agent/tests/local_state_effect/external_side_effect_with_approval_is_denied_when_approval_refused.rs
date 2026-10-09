/// The same `render_chart`-shaped tool is still denied when
/// approval is refused — the approval gate is the binding one, and the
/// external-side-effect gate does not replace it.
#[tokio::test]
async fn external_side_effect_with_approval_is_denied_when_approval_refused() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let provider = OneCallProvider {
        call: ToolCall {
            id: "c1".into(),
            name: "render_chart".into(),
            arguments: serde_json::json!({}),
        },
        turn: Mutex::new(0),
    };
    let sink = RecordingSink {
        events: events.clone(),
    };
    let render_chart = ToolDefinition {
        name: "render_chart".into(),
        description: "visualise a query".into(),
        read_only: true,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: true,
            requires_approval: true,
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
        vec![render_chart],
        AgentLimits::default(),
        &DenyApproval,
        &sink,
        saya_agent::CancellationToken::new(),
    )
    .await
    .expect("a denial is not a turn-ending error");
    assert!(
        calls.lock().unwrap().is_empty(),
        "the tool must not execute when approval is refused"
    );
    let reason = events
        .lock()
        .unwrap()
        .iter()
        .find_map(|event| match event {
            AgentEvent::ToolDenied { name, reason } if name == "render_chart" => {
                Some(reason.clone())
            }
            _ => None,
        })
        .expect("a ToolDenied event must be emitted");
    assert!(
        reason.contains("approval"),
        "the reason must name approval as the refusing gate, got: {reason}"
    );
}
