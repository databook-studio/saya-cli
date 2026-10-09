#[tokio::test]
async fn anthropic_stream_surfaces_cumulative_token_usage() {
    use saya_agent::{AnthropicProvider, ProviderEvent, TokenUsage};
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":100,\"cache_read_input_tokens\":20,\"cache_creation_input_tokens\":30}}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{},\"usage\":{\"output_tokens\":34}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ],
    }]);
    let provider =
        AnthropicProvider::new(ProviderSettings::new("m", Some(base)), Some("k")).unwrap();
    let mut stream = provider
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let events = drain(&mut stream).await;
    handle.join().unwrap();
    let position = events
        .iter()
        .position(|event| matches!(event, ProviderEvent::Usage(usage) if *usage == TokenUsage::new(150, 34).with_cached_input(Some(20)).with_cache_creation(Some(30))))
        .expect("usage event with both counters must arrive");
    assert!(
        matches!(events[position + 1], ProviderEvent::Done),
        "usage precedes Done"
    );
}
