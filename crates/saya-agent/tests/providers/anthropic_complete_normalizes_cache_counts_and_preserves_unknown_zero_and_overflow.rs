#[tokio::test]
async fn anthropic_complete_normalizes_cache_counts_and_preserves_unknown_zero_and_overflow() {
    use saya_agent::{AnthropicProvider, TokenUsage};
    let cases = [
        (
            "{\"input_tokens\":100,\"cache_read_input_tokens\":20,\"cache_creation_input_tokens\":30}",
            TokenUsage::new(150, 7)
                .with_cached_input(Some(20))
                .with_cache_creation(Some(30)),
        ),
        (
            "{\"input_tokens\":0,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}",
            TokenUsage::new(0, 7)
                .with_cached_input(Some(0))
                .with_cache_creation(Some(0)),
        ),
        ("{\"input_tokens\":100}", TokenUsage::new(100, 7)),
        (
            "{\"input_tokens\":18446744073709551615,\"cache_read_input_tokens\":1,\"cache_creation_input_tokens\":1}",
            TokenUsage::new(u64::MAX, 7)
                .with_cached_input(Some(1))
                .with_cache_creation(Some(1)),
        ),
    ];

    for (input_usage, expected) in cases {
        let (base, _, handle) = server(vec![Reply {
            status: 200,
            chunks: vec![
                "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":",
                input_usage,
                "}}\n\n",
                "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\n",
                "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{},\"usage\":{\"output_tokens\":7}}\n\n",
                "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
            ],
        }]);
        let provider =
            AnthropicProvider::new(ProviderSettings::new("m", Some(base)), Some("k")).unwrap();
        let response = provider.complete(request()).await.unwrap();
        handle.join().unwrap();
        assert_eq!(response.usage, Some(expected));
    }
}
