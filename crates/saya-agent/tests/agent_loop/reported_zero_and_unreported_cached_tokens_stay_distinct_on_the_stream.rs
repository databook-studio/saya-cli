/// The wire distinction the whole type exists for: a provider reporting zero
/// cached tokens is saying something, and a provider reporting nothing is not.
/// On the stream the reported zero survives as a number and the unreported
/// serializes `null` — never folded into one another.
#[tokio::test]
async fn reported_zero_and_unreported_cached_tokens_stay_distinct_on_the_stream() {
    use saya_agent::{ProviderEvent, ProviderStream, TokenUsage};

    struct ZeroCacheProvider {
        turns: std::sync::Mutex<usize>,
    }

    #[async_trait::async_trait]
    impl ChatProvider for ZeroCacheProvider {
        fn name(&self) -> &str {
            "zero-cache"
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
            let usage = if turn == 1 {
                // A cold cache, reported as a number.
                TokenUsage::new(10, 5).with_cached_input(Some(0))
            } else {
                // No cache figure at all: the field stays absent.
                TokenUsage::new(10, 5)
            };
            let events = if turn == 1 {
                vec![
                    Ok(ProviderEvent::ToolCalls(vec![ToolCall {
                        id: "call-1".into(),
                        name: "schema_discovery".into(),
                        arguments: serde_json::json!({}),
                    }])),
                    Ok(ProviderEvent::Usage(usage)),
                    Ok(ProviderEvent::Done),
                ]
            } else {
                vec![
                    Ok(ProviderEvent::TextDelta("final answer".into())),
                    Ok(ProviderEvent::Usage(usage)),
                    Ok(ProviderEvent::Done),
                ]
            };
            Ok(Box::pin(futures_util::stream::iter(events)))
        }
    }

    let captured = Arc::new(Mutex::new(Vec::new()));
    run_agent_with_sink(
        &ZeroCacheProvider {
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
    let events = captured.lock().unwrap();
    let reported: Vec<saya_agent::TokenUsage> = events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::Usage { usage, .. } => Some(*usage),
            _ => None,
        })
        .collect();
    assert_eq!(reported.len(), 2, "one event per call: {events:?}");
    assert_eq!(
        reported[0].cached_input_tokens,
        Some(0),
        "a reported zero must survive the stream as a reported zero"
    );
    assert_eq!(
        reported[1].cached_input_tokens, None,
        "an unreported cache figure must stay absent, not become zero"
    );
    // And the serialised forms differ, which is the property a consumer reads.
    assert_ne!(
        serde_json::to_string(&reported[0]).unwrap(),
        serde_json::to_string(&reported[1]).unwrap(),
        "reported zero and unreported must not serialize identically"
    );
}
