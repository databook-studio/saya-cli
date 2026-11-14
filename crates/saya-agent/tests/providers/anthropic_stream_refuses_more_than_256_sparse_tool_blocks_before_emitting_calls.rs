#[tokio::test]
async fn anthropic_stream_refuses_more_than_256_sparse_tool_blocks_before_emitting_calls() {
    use saya_agent::{AnthropicProvider, ProviderEvent};

    let mut body = Vec::new();
    for index in 0..257_u64 {
        writeln!(
            body,
            "event: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":{},\"content_block\":{{\"type\":\"tool_use\",\"id\":\"x\",\"name\":\"y\"}}}}\n",
            index * 1_000_000,
        )
        .unwrap();
    }
    let (base, handle) = byte_server(vec![body]);
    let provider =
        AnthropicProvider::new(ProviderSettings::new("m", Some(base)), Some("k")).unwrap();
    let mut stream = provider
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let mut emitted_calls = false;
    let error = loop {
        match stream.next().await.expect("stream settles") {
            Ok(ProviderEvent::ToolCalls(_)) => emitted_calls = true,
            Ok(_) => {}
            Err(error) => break error,
        }
    };
    handle.join().unwrap();
    assert!(matches!(error, ProviderError::ToolCollectionLimit));
    assert!(
        !emitted_calls,
        "the oversized batch must not become tool calls"
    );
}
