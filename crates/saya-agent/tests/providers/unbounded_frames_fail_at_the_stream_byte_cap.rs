#[tokio::test]
async fn unbounded_frames_fail_at_the_stream_byte_cap() {
    let (base, handle) = byte_server(vec![vec![b'A'; 3 << 20]]);
    let provider = OllamaProvider::new(ProviderSettings::new("test", Some(base))).unwrap();
    let started = std::time::Instant::now();
    let mut stream = provider
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let mut capped = false;
    while let Some(event) = stream.next().await {
        if let Err(error) = event {
            capped = error.to_string().contains("size limit");
            break;
        }
    }
    assert!(capped, "boundary-less frames must trip the stream cap");
    assert!(started.elapsed() < Duration::from_secs(5));
    handle.join().unwrap();
}
