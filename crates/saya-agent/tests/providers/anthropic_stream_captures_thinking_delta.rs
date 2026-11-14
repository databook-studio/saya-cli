/// `thinking_delta.thinking` is parsed
/// into a `ReasoningDelta` event and threaded onto `ChatResponse.reasoning`.
#[tokio::test]
async fn anthropic_stream_captures_thinking_delta() {
    use saya_agent::{AnthropicProvider, ProviderEvent};
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"the column is nullable\"}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ],
    }]);
    let provider =
        AnthropicProvider::new(ProviderSettings::new("m", Some(base)), Some("k")).unwrap();
    let response = provider.complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(response.message.content, "ok");
    assert_eq!(
        response.reasoning.as_deref(),
        Some("the column is nullable"),
        "thinking_delta.thinking must reach ChatResponse.reasoning"
    );

    // Observe the ReasoningDelta event directly.
    let (base2, _, handle2) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"t\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ],
    }]);
    let provider =
        AnthropicProvider::new(ProviderSettings::new("m", Some(base2)), Some("k")).unwrap();
    let mut stream = provider
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let events = drain_reasoning(&mut stream).await;
    handle2.join().unwrap();
    assert!(
        events.contains(&ProviderEvent::ReasoningDelta("t".into())),
        "thinking_delta must emit a ReasoningDelta: {events:?}"
    );
}
