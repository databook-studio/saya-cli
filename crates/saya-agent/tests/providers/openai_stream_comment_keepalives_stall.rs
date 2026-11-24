/// Serves one streaming request: the `content` frames once, then `keepalive`
/// `repeats` times at `interval`. The pause is what makes the keep-alive a
/// transport chunk rather than a gap: the client sees data and never progress.
fn content_then_keepalive_server(
    content: &'static str,
    keepalive: &'static str,
    interval: Duration,
    repeats: usize,
) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let body_len = content.len() + keepalive.len() * repeats;
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_request(&mut stream);
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {body_len}\r\nContent-Type: text/event-stream\r\nConnection: keep-alive\r\n\r\n"
        );
        if stream.write_all(content.as_bytes()).is_err() {
            return;
        }
        let _ = stream.flush();
        for _ in 0..repeats {
            if stream.write_all(keepalive.as_bytes()).is_err() {
                return;
            }
            if stream.flush().is_err() {
                return;
            }
            thread::sleep(interval);
        }
    });
    (base, handle)
}

/// An SSE comment line (`: OPENROUTER PROCESSING`) is a transport keep-alive,
/// not model output: after one content delta, a stream that sends only comment
/// lines must be cut as stalled within the progress window, not held open.
#[tokio::test]
async fn openai_stream_stalls_after_content_before_comment_keepalives() {
    let (base, handle) = content_then_keepalive_server(
        "data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n\n",
        ": OPENROUTER PROCESSING\n\n",
        Duration::from_millis(40),
        40,
    );
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
    let error = error.expect("a comment-keep-alive stream must be aborted");
    assert!(
        matches!(error, ProviderError::StreamStalled),
        "the stall is its own class, not an idle or transport failure: {error:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the progress timer, not the idle timeout or socket close, ends the stream"
    );
    handle.join().unwrap();
}
