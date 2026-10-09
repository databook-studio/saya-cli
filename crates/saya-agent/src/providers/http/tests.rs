use super::*;
use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER};

#[test]
fn test_parse_retry_after_valid_seconds() {
    let mut headers = HeaderMap::new();
    headers.insert(RETRY_AFTER, HeaderValue::from_static("5"));
    assert_eq!(parse_retry_after(&headers), Some(Duration::from_secs(5)));

    headers.insert(RETRY_AFTER, HeaderValue::from_static(" 120 "));
    assert_eq!(parse_retry_after(&headers), Some(Duration::from_secs(120)));
}

#[test]
fn test_parse_retry_after_invalid_or_missing() {
    let mut headers = HeaderMap::new();
    assert_eq!(parse_retry_after(&headers), None);

    headers.insert(RETRY_AFTER, HeaderValue::from_static(""));
    assert_eq!(parse_retry_after(&headers), None);

    headers.insert(
        RETRY_AFTER,
        HeaderValue::from_static("Mon, 01 Jan 2030 00:00:00 GMT"),
    );
    assert_eq!(parse_retry_after(&headers), None);

    headers.insert(RETRY_AFTER, HeaderValue::from_static("invalid"));
    assert_eq!(parse_retry_after(&headers), None);

    headers.insert(RETRY_AFTER, HeaderValue::from_static("-5"));
    assert_eq!(parse_retry_after(&headers), None);
}

#[test]
fn test_jitter_bounds() {
    let bases = [
        Duration::ZERO,
        Duration::from_millis(1),
        Duration::from_millis(100),
        Duration::from_secs(1),
        Duration::from_secs(10),
        Duration::from_secs(120),
    ];

    for base in bases {
        for _ in 0..50 {
            let j = jitter(base);
            assert!(
                j >= base / 2,
                "jitter({base:?}) = {j:?} should be >= base / 2 ({:?})",
                base / 2
            );
            assert!(
                j <= base,
                "jitter({base:?}) = {j:?} should be <= base ({base:?})"
            );
        }
    }
}

#[test]
fn test_retry_after_and_backoff_capped_at_max() {
    let mut headers = HeaderMap::new();
    headers.insert(RETRY_AFTER, HeaderValue::from_static("120"));
    let parsed = parse_retry_after(&headers).unwrap();
    let delay = parsed.min(MAX_BACKOFF);
    assert_eq!(delay, MAX_BACKOFF);

    let large_base = Duration::from_secs(200);
    let j = jitter(large_base).min(MAX_BACKOFF);
    assert!(j <= MAX_BACKOFF);
}

#[test]
fn status_descriptions_name_the_fix_without_the_body() {
    let text = describe(StatusCode::UNAUTHORIZED);
    assert!(text.contains("401") && text.contains("API key"), "{text}");
    let not_found = describe(StatusCode::NOT_FOUND);
    assert!(
        not_found.contains("404") && not_found.contains("model"),
        "{not_found}"
    );
    let payload = describe(StatusCode::PAYLOAD_TOO_LARGE);
    assert!(
        payload.contains("413") && payload.contains("too large"),
        "{payload}"
    );
    let other = describe(StatusCode::FAILED_DEPENDENCY);
    assert!(text.len() > 10 && !other.is_empty());
}

#[tokio::test]
async fn test_send_stream_honors_retry_after() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server_task = tokio::spawn(async move {
        if let Ok((socket, _)) = listener.accept().await {
            let mut buf = [0_u8; 1024];
            let _ = socket.readable().await;
            let _ = socket.try_read(&mut buf);
            let response = "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            let _ = socket.writable().await;
            let _ = socket.try_write(response.as_bytes());
        }
        if let Ok((socket, _)) = listener.accept().await {
            let mut buf = [0_u8; 1024];
            let _ = socket.readable().await;
            let _ = socket.try_read(&mut buf);
            let response = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
            let _ = socket.writable().await;
            let _ = socket.try_write(response.as_bytes());
        }
    });

    let client = reqwest::Client::new();
    let cancellation = CancellationToken::new();
    let url = format!("http://{addr}");
    let delays = vec![Duration::from_secs(10)];

    let start = std::time::Instant::now();
    let res = send_stream(
        || client.get(&url),
        &delays,
        &cancellation,
        &url,
        Duration::from_secs(5),
    )
    .await;
    let elapsed = start.elapsed();

    assert!(res.is_ok());
    assert_eq!(res.unwrap().status(), reqwest::StatusCode::OK);
    assert!(elapsed < Duration::from_secs(2));

    server_task.await.unwrap();
}

#[tokio::test]
async fn send_stream_times_out_while_establishing() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        drop(socket);
    });

    let client = reqwest::Client::new();
    let cancellation = CancellationToken::new();
    let url = format!("http://{addr}");
    let started = std::time::Instant::now();
    let error = send_stream(
        || client.get(&url),
        &[],
        &cancellation,
        &url,
        Duration::from_millis(20),
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("timed out"), "{error:?}");
    assert!(started.elapsed() < Duration::from_millis(150));
    server_task.await.unwrap();
}

#[tokio::test]
async fn send_stream_cancellation_wins_before_establishment() {
    let client = reqwest::Client::new();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let error = send_stream(
        || client.get("http://127.0.0.1:1"),
        &[],
        &cancellation,
        "http://127.0.0.1:1",
        Duration::from_secs(5),
    )
    .await
    .unwrap_err();

    assert_eq!(error, ProviderError::Cancelled);
}
