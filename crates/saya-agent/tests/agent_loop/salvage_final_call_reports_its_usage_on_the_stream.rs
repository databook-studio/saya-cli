/// A salvaged run's final call produces the answer the reader sees, so its
/// usage reaches the stream as an answering call — the salvage path drives its
/// last call through the same `receive` the loop uses.
#[tokio::test]
async fn salvage_final_call_reports_its_usage_on_the_stream() {
    use saya_agent::{ProviderEvent, ProviderStream, TokenUsage, UsageCall};

    struct SalvageUsageProvider {
        turns: std::sync::Mutex<usize>,
    }

    #[async_trait::async_trait]
    impl ChatProvider for SalvageUsageProvider {
        fn name(&self) -> &str {
            "salvage-usage"
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
            let turn = {
                let mut turns = self.turns.lock().unwrap();
                *turns += 1;
                *turns
            };
            let events = if turn == 1 {
                vec![
                    Ok(ProviderEvent::ToolCalls(vec![ToolCall {
                        id: "call-1".into(),
                        name: "bounded_sql_query".into(),
                        arguments: serde_json::json!({"sql":"select 1"}),
                    }])),
                    Ok(ProviderEvent::Usage(TokenUsage::new(20, 4))),
                    Ok(ProviderEvent::Done),
                ]
            } else {
                // The salvage call: no tools, its own report.
                vec![
                    Ok(ProviderEvent::TextDelta("salvaged answer".into())),
                    Ok(ProviderEvent::Usage(TokenUsage::new(30, 6))),
                    Ok(ProviderEvent::Done),
                ]
            };
            Ok(Box::pin(futures_util::stream::iter(events)))
        }
    }

    let captured = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent_with_sink(
        &SalvageUsageProvider {
            turns: std::sync::Mutex::new(0),
        },
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits {
            max_turns: Some(1),
            ..AgentLimits::default()
        },
        &AllowReadOnlyApproval,
        &RecordingSink {
            events: captured.clone(),
        },
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(output.answer, "salvaged answer");
    assert!(output.truncated, "the run was salvaged, not completed");
    let events = captured.lock().unwrap();
    let reported: Vec<(UsageCall, u64)> = events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::Usage { call, usage } => Some((*call, usage.input_tokens)),
            _ => None,
        })
        .collect();
    assert_eq!(
        reported,
        vec![(UsageCall::Answer, 20), (UsageCall::Answer, 30)],
        "the salvage call is an answering call and reports its usage: {events:?}"
    );
}
