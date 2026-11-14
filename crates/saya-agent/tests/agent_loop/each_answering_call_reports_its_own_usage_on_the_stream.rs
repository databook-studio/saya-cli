/// Usage reaches the event stream too: one `Usage` event per provider call
/// that reported any, carrying **that call's** counts — not the run total, so a
/// consumer can sum them and keep the answer's cost apart from anything else.
#[tokio::test]
async fn each_answering_call_reports_its_own_usage_on_the_stream() {
    use saya_agent::{ProviderEvent, ProviderStream, TokenUsage, UsageCall};

    struct UsageProvider {
        turns: std::sync::Mutex<usize>,
    }

    #[async_trait::async_trait]
    impl ChatProvider for UsageProvider {
        fn name(&self) -> &str {
            "usage"
        }
        async fn complete(
            &self,
            _: ChatRequest,
        ) -> Result<ChatResponse, saya_agent::ProviderError> {
            panic!("loop must drive providers through stream()");
        }
        async fn stream(
            &self,
            _: ChatRequest,
            _: CancellationToken,
        ) -> Result<ProviderStream, saya_agent::ProviderError> {
            let mut turns = self.turns.lock().unwrap();
            *turns += 1;
            let turn = *turns;
            drop(turns);
            let events = if turn == 1 {
                vec![
                    Ok(ProviderEvent::ToolCalls(vec![ToolCall {
                        id: "call-1".into(),
                        name: "schema_discovery".into(),
                        arguments: serde_json::json!({}),
                    }])),
                    Ok(ProviderEvent::Usage(TokenUsage::new(3, 7))),
                    Ok(ProviderEvent::Done),
                ]
            } else {
                vec![
                    Ok(ProviderEvent::TextDelta("final answer".into())),
                    Ok(ProviderEvent::Usage(TokenUsage::new(5, 9))),
                    Ok(ProviderEvent::Done),
                ]
            };
            Ok(Box::pin(futures_util::stream::iter(events)))
        }
    }

    let captured = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent_with_sink(
        &UsageProvider {
            turns: std::sync::Mutex::new(0),
        },
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &RecordingSink {
            events: captured.clone(),
        },
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(output.answer, "final answer");
    let events = captured.lock().unwrap();
    let reported: Vec<(UsageCall, saya_agent::TokenUsage)> = events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::Usage { call, usage } => Some((*call, *usage)),
            _ => None,
        })
        .collect();
    // One event per call, each carrying that call's own counts — a snapshot,
    // never the running total (which is `8/16` and belongs to `AgentOutput`).
    assert_eq!(
        reported.len(),
        2,
        "one event per answering call: {events:?}"
    );
    assert_eq!(reported[0], (UsageCall::Answer, TokenUsage::new(3, 7)));
    assert_eq!(reported[1], (UsageCall::Answer, TokenUsage::new(5, 9)));
}
