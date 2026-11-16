#[tokio::test]
async fn anthropic_stream_keeps_a_valid_id_with_non_object_arguments() {
    use saya_agent::{AnthropicProvider, ProviderEvent};

    let body = b"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"call-1\",\"name\":\"schema_discovery\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"not-an-object\\\"\"}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
    let (base, handle) = byte_server(vec![body.to_vec()]);
    let provider =
        AnthropicProvider::new(ProviderSettings::new("m", Some(base)), Some("k")).unwrap();
    let mut stream = provider
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let calls = loop {
        if let ProviderEvent::ToolCalls(calls) = stream
            .next()
            .await
            .expect("stream settles")
            .expect("provider accepts call")
        {
            break calls;
        }
    };
    handle.join().unwrap();
    assert_eq!(calls[0].id, "call-1");
    assert_eq!(calls[0].arguments, serde_json::json!("not-an-object"));
}
