/// A usage-only frame is a transport chunk, not model progress: a stream that
/// sends nothing else must still be aborted as stalled. The idle timeout is
/// deliberately longer than the stall window, so only the progress timer can
/// end this stream.
#[tokio::test]
async fn openai_stream_stalls_on_a_usage_only_keepalive_stream() {
    let frames = vec!["data: {\"choices\":[],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":0}}\n\n"];
    let (base, handle) = paced_server(frames, Duration::from_millis(40), 12);
    let mut settings = ProviderSettings::new("test-model", Some(format!("{base}/v1")))
        .with_retry_delays(vec![Duration::ZERO])
        .with_idle_timeout(Duration::from_secs(5));
    settings.stall_timeout = Duration::from_millis(150);
    let provider = OpenAiCompatibleProvider::new(settings, Some("secret-sentinel")).unwrap();
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
    let error = error.expect("a keep-alive-only stream must be aborted");
    assert!(
        matches!(error, ProviderError::StreamStalled),
        "the stall is its own class, not an idle or transport failure: {error:?}"
    );
    assert!(
        started.elapsed() >= Duration::from_millis(150),
        "the stall window must elapse before the cut"
    );
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the progress timer, not the idle timeout or socket close, ends the stream"
    );
    handle.join().unwrap();
}

/// Slow but steady reasoning deltas are progress: a gap under the stall window
/// between them must never abort the request, even when the whole stream lasts
/// longer than the window.
#[tokio::test]
async fn openai_stream_keeps_a_slow_steady_reasoning_stream_alive() {
    let frames = vec![
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"a\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"b\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"c\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"d\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"answer\"}}]}\n\n",
        "data: [DONE]\n\n",
    ];
    let (base, handle) = paced_server(frames, Duration::from_millis(50), 1);
    let mut settings = ProviderSettings::new("test-model", Some(format!("{base}/v1")))
        .with_retry_delays(vec![Duration::ZERO])
        .with_idle_timeout(Duration::from_secs(5));
    settings.stall_timeout = Duration::from_millis(150);
    let provider = OpenAiCompatibleProvider::new(settings, Some("secret-sentinel")).unwrap();
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
    assert!(reasoning, "the reasoning deltas reached the caller");
    handle.join().unwrap();
}
