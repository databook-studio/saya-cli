#[tokio::test]
async fn openai_sse_ping_frames_stop_at_the_shared_frame_limit() {
    const FRAME_LIMIT: usize = 65_536;
    let mut body = b": ping\n\n".repeat(FRAME_LIMIT + 1);
    body.splice(
        0..0,
        b"data: {\"choices\":[],\"usage\":{\"prompt_tokens\":40,\"completion_tokens\":10}}\n\n"
            .iter()
            .copied(),
    );
    body.extend_from_slice(b"data: [DONE]\n\n");
    let (base, handle) = byte_server(vec![body]);

    let provider = openai(base);
    let mut stream = provider
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let mut latest_usage = None;
    let error = loop {
        match stream.next().await.unwrap() {
            Ok(ProviderEvent::Usage(usage)) => latest_usage = Some(usage),
            Ok(event) => panic!("over-limit parser emitted an unexpected event: {event:?}"),
            Err(error) => break error,
        }
    };
    handle.join().unwrap();

    assert_eq!(latest_usage, Some(TokenUsage::new(40, 10)));
    assert_eq!(
        error,
        ProviderError::StreamItemLimit {
            usage: Some(Box::new(TokenUsage::new(40, 10)))
        }
    );
}
