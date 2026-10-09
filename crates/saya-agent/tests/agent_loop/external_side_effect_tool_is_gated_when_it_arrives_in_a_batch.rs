/// The gated tool arriving in a batch with an auto-runnable call must still be
/// gated: the batch is not run concurrently for it, it falls through to the
/// sequential path and is denied, while the auto-runnable sibling executes.
/// Against the code before parallel tool calls the batch predicate already excluded the gated
/// tool (it tests `external_side_effect`), so the batch fell through — but the
/// sequential path then *ran* it, since it ignored `external_side_effect`. So
/// the "must not execute" assertion fails there.
#[tokio::test]
async fn external_side_effect_tool_is_gated_when_it_arrives_in_a_batch() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let provider = MockProvider {
        responses: Mutex::new(vec![
            ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![
                    ToolCall {
                        id: "call-a".into(),
                        name: "open_browser".into(),
                        arguments: serde_json::json!({}),
                    },
                    ToolCall {
                        id: "call-b".into(),
                        name: "schema_discovery".into(),
                        arguments: serde_json::json!({}),
                    },
                ],
                tool_call_id: None,
            }),
            ChatResponse::new(ChatMessage::text("assistant", "done")),
        ]),
    };
    let definitions = {
        let mut defs = definitions();
        defs.push(external_side_effect_without_approval_tool());
        defs
    };
    let sink = RecordingSink {
        events: events.clone(),
    };
    let _ = run_agent_with_sink(
        &provider,
        &MockTools {
            calls: calls.clone(),
        },
        request(),
        definitions,
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &sink,
        CancellationToken::new(),
    )
    .await
    .expect("run completes");
    let calls = calls.lock().unwrap().clone();
    let events = events.lock().unwrap().clone();
    assert_open_browser_gated(&calls, &events);
    // The auto-runnable sibling is unaffected: it executes normally.
    assert!(
        calls.iter().any(|name| name == "schema_discovery"),
        "the non-gated sibling must still execute; got calls {calls:?}"
    );
}
