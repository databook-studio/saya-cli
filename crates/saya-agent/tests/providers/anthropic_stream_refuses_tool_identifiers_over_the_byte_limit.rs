#[tokio::test]
async fn anthropic_stream_refuses_tool_identifiers_over_the_byte_limit() {
    use saya_agent::AnthropicProvider;

    let id = "x".repeat(saya_agent::MAX_STREAM_BYTES / 2 + 1);
    let first = format!(
        "event: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":1,\"content_block\":{{\"type\":\"tool_use\",\"id\":\"{id}\",\"name\":\"x\"}}}}\n\n"
    );
    let second = format!(
        "event: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":1000000,\"content_block\":{{\"type\":\"tool_use\",\"id\":\"{id}\",\"name\":\"x\"}}}}\n\n"
    );
    let (base, handle) = byte_server(vec![first.into_bytes(), second.into_bytes()]);
    let provider =
        AnthropicProvider::new(ProviderSettings::new("m", Some(base)), Some("k")).unwrap();
    let mut stream = provider
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let error = loop {
        match stream.next().await.expect("stream settles") {
            Ok(_) => {}
            Err(error) => break error,
        }
    };
    handle.join().unwrap();
    assert!(matches!(error, ProviderError::ToolCollectionLimit));
}
