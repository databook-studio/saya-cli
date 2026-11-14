async fn run_policy_turn(
    call_count: usize,
    effect: ToolEffect,
    concurrency: ToolConcurrency,
    limits: AgentLimits,
    approval: &dyn ApprovalDecider,
) -> (usize, Vec<String>, Vec<String>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let provider = PolicyProvider {
        calls: (0..call_count)
            .map(|index| ToolCall {
                id: format!("policy-{index}"),
                name: "policy_tool".into(),
                arguments: serde_json::json!({}),
            })
            .collect(),
        turn: Mutex::new(0),
    };
    let output = run_agent_with_sink(
        &provider,
        &RecordingExecutor {
            calls: calls.clone(),
        },
        request(),
        vec![policy_tool(effect, concurrency)],
        limits,
        approval,
        &RecordingSink {
            events: events.clone(),
        },
        saya_agent::CancellationToken::new(),
    )
    .await
    .expect("policy refusals feed results and the turn completes");
    let reasons = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            AgentEvent::ToolDenied { reason, .. } => Some(reason.clone()),
            _ => None,
        })
        .collect();
    let statuses = output
        .tool_metadata
        .into_iter()
        .map(|metadata| metadata.status)
        .collect();
    (calls.lock().unwrap().len(), reasons, statuses)
}
