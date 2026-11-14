/// a stream with no `thinking` block
/// leaves `ChatResponse.reasoning` `None` — reasoning not reported, no error.
#[tokio::test]
async fn anthropic_stream_without_thinking_leaves_reasoning_none() {
    use saya_agent::AnthropicProvider;
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ],
    }]);
    let provider =
        AnthropicProvider::new(ProviderSettings::new("m", Some(base)), Some("k")).unwrap();
    let response = provider.complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(response.message.content, "ok");
    assert_eq!(response.reasoning, None);
}
