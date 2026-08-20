use crate::{CancellationToken, ProviderError};
use reqwest::{RequestBuilder, Response, StatusCode};
use std::time::Duration;

const MAX_BACKOFF: Duration = Duration::from_secs(60);

pub(super) async fn send_stream(
    mut build: impl FnMut() -> RequestBuilder,
    delays: &[Duration],
    cancellation: &CancellationToken,
) -> Result<Response, ProviderError> {
    for attempt in 0..=delays.len() {
        let response = tokio::select! {
            _ = cancellation.cancelled() => return Err(ProviderError::Cancelled),
            response = build().send() => response,
        };
        match response {
            Ok(response) if response.status().is_success() => return Ok(response),
            Ok(response) if retryable(response.status()) && attempt < delays.len() => {
                let delay = parse_retry_after(response.headers())
                    .unwrap_or_else(|| jitter(delays[attempt]))
                    .min(MAX_BACKOFF);
                wait(delay, cancellation).await?;
            }
            Ok(response) => {
                return Err(ProviderError::Request(format!(
                    "HTTP {}",
                    response.status().as_u16()
                )));
            }
            Err(_) if attempt < delays.len() => {
                let delay = jitter(delays[attempt]).min(MAX_BACKOFF);
                wait(delay, cancellation).await?;
            }
            Err(_) => return Err(ProviderError::Request("network request failed".into())),
        }
    }
    Err(ProviderError::Request("network request failed".into()))
}

async fn wait(delay: Duration, cancellation: &CancellationToken) -> Result<(), ProviderError> {
    tokio::select! {
        _ = cancellation.cancelled() => Err(ProviderError::Cancelled),
        _ = tokio::time::sleep(delay) => Ok(()),
    }
}

fn retryable(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

/// Parses the `Retry-After` header as integer delta-seconds.
///
/// HTTP-date formats or unparseable values return `None`.
fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let value = headers.get(reqwest::header::RETRY_AFTER)?;
    let str_val = value.to_str().ok()?.trim();
    let seconds: u64 = str_val.parse().ok()?;
    Some(Duration::from_secs(seconds))
}

/// Computes a jittered duration uniformly distributed in `[base / 2, base]`.
///
/// Uses sub-second nanoseconds from system time as a non-cryptographic source
/// of pseudo-randomness to decorrelate retry attempts.
fn jitter(base: Duration) -> Duration {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let f = 0.5 + 0.5 * (f64::from(nanos) / 1_000_000_000.0);
    base.mul_f64(f)
}

#[cfg(test)]
mod tests {
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
                let response =
                    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
                let _ = socket.writable().await;
                let _ = socket.try_write(response.as_bytes());
            }
        });

        let client = reqwest::Client::new();
        let cancellation = CancellationToken::new();
        let url = format!("http://{addr}");
        let delays = vec![Duration::from_secs(10)];

        let start = std::time::Instant::now();
        let res = send_stream(|| client.get(&url), &delays, &cancellation).await;
        let elapsed = start.elapsed();

        assert!(res.is_ok());
        assert_eq!(res.unwrap().status(), reqwest::StatusCode::OK);
        assert!(elapsed < Duration::from_secs(2));

        server_task.await.unwrap();
    }
}
