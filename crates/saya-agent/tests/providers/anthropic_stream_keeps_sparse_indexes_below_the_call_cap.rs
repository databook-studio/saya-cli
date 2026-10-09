#[tokio::test]
async fn anthropic_stream_keeps_sparse_indexes_below_the_call_cap() {
    use saya_agent::AnthropicProvider;

    let body = b"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"one\",\"name\":\"schema_discovery\"}}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1000000,\"content_block\":{\"type\":\"tool_use\",\"id\":\"two\",\"name\":\"schema_discovery\"}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
    let (base, handle) = byte_server(vec![body.to_vec()]);
    let provider =
        AnthropicProvider::new(ProviderSettings::new("m", Some(base)), Some("k")).unwrap();
    let response = provider.complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(response.message.tool_calls.len(), 2);
    assert_eq!(response.message.tool_calls[0].id, "one");
    assert_eq!(response.message.tool_calls[1].id, "two");
}
