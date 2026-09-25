use crate::{CancellationToken, ProviderError};
use futures_util::StreamExt;
use reqwest::{RequestBuilder, Response, StatusCode};
use serde::de::DeserializeOwned;
use std::time::Duration;

const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Decode one HTTP JSON body without allowing reqwest to buffer an
/// unbounded response before the provider applies its stream budget.
pub(super) async fn read_json<T: DeserializeOwned>(
    response: Response,
    max_bytes: usize,
) -> Result<T, ProviderError> {
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| ProviderError::InvalidResponse)?;
        if bytes.len().saturating_add(chunk.len()) > max_bytes {
            return Err(ProviderError::Request(
                "provider response exceeded size limit".into(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| ProviderError::InvalidResponse)
}

enum AttemptError {
    Network,
    Timeout,
}

pub(super) async fn send_stream(
    mut build: impl FnMut() -> RequestBuilder,
    delays: &[Duration],
    cancellation: &CancellationToken,
    endpoint: &str,
    establishment_timeout: Duration,
) -> Result<Response, ProviderError> {
    for attempt in 0..=delays.len() {
        let response = tokio::select! {
            _ = cancellation.cancelled() => return Err(ProviderError::Cancelled),
            response = tokio::time::timeout(establishment_timeout, build().send()) => match response {
                Ok(Ok(response)) => Ok(response),
                Ok(Err(_)) => Err(AttemptError::Network),
                Err(_) => Err(AttemptError::Timeout),
            },
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
                return Err(ProviderError::Request(describe(response.status())));
            }
            Err(error) if attempt < delays.len() => {
                let delay = jitter(delays[attempt]).min(MAX_BACKOFF);
                wait(delay, cancellation).await?;
                let _ = error;
            }
            Err(error) => {
                if matches!(error, AttemptError::Timeout) {
                    return Err(ProviderError::Request(format!(
                        "provider request timed out while establishing a connection to {endpoint}"
                    )));
                }
                return Err(ProviderError::Request(format!(
                    "could not reach the provider at {endpoint} — check that it is running and the configured base_url is correct"
                )));
            }
        }
    }
    Err(ProviderError::Request(format!(
        "could not reach the provider at {endpoint} — check that it is running and the configured base_url is correct"
    )))
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

/// Turns a provider's status code into a redacted but *diagnosable* message:
/// the code plus the fix a user can act on, never the response body (which
/// may echo account details).
fn describe(status: StatusCode) -> String {
    let code = status.as_u16();
    let hint = match code {
        401 | 403 => "authentication failed — check the API key configured for this provider",
        402 => "the provider requires payment or quota for this request",
        404 => "endpoint or model not found — check the configured model and base_url",
        413 => "request too large — shorten the prompt or clear context",
        400 => "request rejected by the provider — check the model name and parameters",
        _ => "provider rejected the request",
    };
    format!("HTTP {code}: {hint}")
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
}
