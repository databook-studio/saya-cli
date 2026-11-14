/// Cache and reasoning counts must survive the run, not just the turn. Each is
/// optional because a provider that never reports one has to stay
/// distinguishable from a cold cache — so the run total stays `None` until some
/// turn reports, and from then on sums only what was actually reported. A turn
/// that reports nothing must not fold in as a zero.
#[tokio::test]
async fn optional_usage_counts_survive_the_run_and_absent_stays_absent() {
    use saya_agent::{ProviderEvent, ProviderStream, TokenUsage};

    struct DetailedUsageProvider {
        turns: std::sync::Mutex<usize>,
    }

    #[async_trait::async_trait]
    impl ChatProvider for DetailedUsageProvider {
        fn name(&self) -> &str {
            "detailed-usage"
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
                    Ok(ProviderEvent::Usage(
                        TokenUsage::new(3, 7)
                            .with_cached_input(Some(2))
                            .with_reasoning(Some(4)),
                    )),
                    Ok(ProviderEvent::Done),
                ]
            } else {
                // A second turn that reports no cache detail at all: it must not
                // reset what the first turn reported, nor count as a zero.
                vec![
                    Ok(ProviderEvent::TextDelta("final answer".into())),
                    Ok(ProviderEvent::Usage(
                        TokenUsage::new(5, 9).with_cached_input(Some(6)),
                    )),
                    Ok(ProviderEvent::Done),
                ]
            };
            Ok(Box::pin(futures_util::stream::iter(events)))
        }
    }

    let output = run_agent(
        &DetailedUsageProvider {
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

    assert_eq!(
        output.usage.cached_input_tokens,
        Some(8),
        "cache reads reported across turns must reach the run total"
    );
    assert_eq!(
        output.usage.reasoning_tokens,
        Some(4),
        "a count reported by one turn must survive a later turn that reports none"
    );
    assert_eq!(
        output.usage.cache_creation_input_tokens, None,
        "a count no turn reported must stay absent, not become zero"
    );
}
