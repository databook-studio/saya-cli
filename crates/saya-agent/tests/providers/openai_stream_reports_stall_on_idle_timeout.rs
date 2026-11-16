#[tokio::test]
async fn openai_stream_reports_stall_on_idle_timeout() {
    let body = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n";
    let (base, handle) = keep_open_server(body);
    let provider = OpenAiCompatibleProvider::new(
        ProviderSettings::new("test-model", Some(format!("{base}/v1")))
            .with_retry_delays(vec![Duration::ZERO])
            .with_idle_timeout(Duration::from_millis(120)),
        Some("secret-sentinel"),
    )
    .unwrap();
    let started = std::time::Instant::now();
    let mut stream = provider
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let mut stalled = false;
    while let Some(event) = stream.next().await {
        if let Err(error) = event {
            stalled = format!("{error:?}").contains("stalled");
            break;
        }
    }
    assert!(stalled, "stall must surface as a stall error");
    assert!(started.elapsed() < Duration::from_secs(5));
    handle.join().unwrap();
}
