/// Providers disclose cumulative counts per response; the run total must sum
/// them across turns and land in AgentOutput.
#[tokio::test]
async fn token_usage_sums_across_turns_into_the_output() {
    use saya_agent::{ProviderEvent, ProviderStream, TokenUsage};

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

    let output = run_agent(
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
    )
    .await
    .unwrap();
    assert_eq!(output.answer, "final answer");
    assert_eq!(output.usage, TokenUsage::new(8, 16));
}
