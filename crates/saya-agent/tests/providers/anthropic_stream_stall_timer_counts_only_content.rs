/// A ping is a transport keep-alive, not model progress: a stream that sends
/// nothing else must still be aborted as stalled.
#[tokio::test]
async fn anthropic_stream_stalls_on_a_ping_only_stream() {
    use saya_agent::AnthropicProvider;
    let frames = vec!["event: ping\ndata: {\"type\":\"ping\"}\n\n"];
    let (base, handle) = paced_server(frames, Duration::from_millis(40), 12);
    let mut settings = ProviderSettings::new("test-model", Some(base))
        .with_retry_delays(vec![Duration::ZERO])
        .with_idle_timeout(Duration::from_secs(5));
    settings.stall_timeout = Duration::from_millis(150);
    let provider = AnthropicProvider::new(settings, Some("secret-sentinel")).unwrap();
    let started = std::time::Instant::now();
    let mut stream = provider
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let mut error = None;
    while let Some(event) = stream.next().await {
        if let Err(failure) = event {
            error = Some(failure);
            break;
        }
    }
    let error = error.expect("a ping-only stream must be aborted");
    assert!(
        matches!(error, ProviderError::StreamStalled),
        "the stall is its own class: {error:?}"
    );
    assert!(
        started.elapsed() >= Duration::from_millis(150),
        "the stall window must elapse before the cut"
    );
    assert!(started.elapsed() < Duration::from_secs(3));
    handle.join().unwrap();
}

/// Slow but steady thinking deltas keep the request alive past the stall
/// window, because each delta is progress.
#[tokio::test]
async fn anthropic_stream_keeps_a_slow_steady_thinking_stream_alive() {
    use saya_agent::AnthropicProvider;
    let frames = vec![
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"a\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"b\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"c\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"d\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"answer\"}}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    ];
    let (base, handle) = paced_server(frames, Duration::from_millis(50), 1);
    let mut settings = ProviderSettings::new("test-model", Some(base))
        .with_retry_delays(vec![Duration::ZERO])
        .with_idle_timeout(Duration::from_secs(5));
    settings.stall_timeout = Duration::from_millis(150);
    let provider = AnthropicProvider::new(settings, Some("secret-sentinel")).unwrap();
    let mut stream = provider
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let mut reasoning = false;
    while let Some(event) = stream.next().await {
        match event {
            Ok(ProviderEvent::ReasoningDelta(_)) => reasoning = true,
            Ok(_) => {}
            Err(error) => panic!("a steady stream must not stall: {error:?}"),
        }
    }
    assert!(reasoning, "the thinking deltas reached the caller");
    handle.join().unwrap();
}
