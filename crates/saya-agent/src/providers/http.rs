use crate::{
    CancellationToken, ControlledProviderError, EffectOutcome, PhysicalAttemptControl,
    ProviderError,
};
use futures_util::StreamExt;
use reqwest::{RequestBuilder, Response, StatusCode};
use serde::de::DeserializeOwned;
use std::time::Duration;

const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// What the transport needs to decide whether a rejected send is retryable:
/// the configured backoff schedule.
#[derive(Clone, Copy)]
pub(super) struct RetryPolicy<'a> {
    delays: &'a [Duration],
}

impl<'a> RetryPolicy<'a> {
    pub(super) const fn new(delays: &'a [Duration]) -> Self {
        Self { delays }
    }
}

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
    /// The send failed below HTTP. Only a reset/aborted connection is worth
    /// retrying: a refused or otherwise failed connection would fail the same
    /// way again, and a timeout already spent its establishment budget.
    Network {
        connection_reset: bool,
    },
    Timeout,
}

pub(super) async fn send_stream(
    build: impl FnMut() -> RequestBuilder,
    retry: RetryPolicy<'_>,
    cancellation: &CancellationToken,
    endpoint: &str,
    establishment_timeout: Duration,
) -> Result<Response, ProviderError> {
    send_stream_inner(
        build,
        retry,
        cancellation,
        endpoint,
        establishment_timeout,
        None,
    )
    .await
    .map_err(provider_error)
}

pub(super) async fn send_stream_controlled(
    build: impl FnMut() -> RequestBuilder,
    retry: RetryPolicy<'_>,
    cancellation: &CancellationToken,
    endpoint: &str,
    establishment_timeout: Duration,
    attempts: &mut PhysicalAttemptControl<'_>,
) -> Result<Response, ControlledProviderError> {
    send_stream_inner(
        build,
        retry,
        cancellation,
        endpoint,
        establishment_timeout,
        Some(attempts),
    )
    .await
}

async fn send_stream_inner(
    mut build: impl FnMut() -> RequestBuilder,
    retry: RetryPolicy<'_>,
    cancellation: &CancellationToken,
    endpoint: &str,
    establishment_timeout: Duration,
    mut attempts: Option<&mut PhysicalAttemptControl<'_>>,
) -> Result<Response, ControlledProviderError> {
    let delays = retry.delays;
    for attempt in 0..=delays.len() {
        if let Some(control) = attempts.as_deref_mut() {
            control.admit_before_send().await?;
            if cancellation.is_cancelled() {
                // The loop settles whatever this admission left live. Settling
                // here would race the loop dropping this future on the same
                // cancellation, orphaning the permit.
                return Err(ProviderError::Cancelled.into());
            }
        }
        let response = tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                return Err(ProviderError::Cancelled.into());
            }
            response = tokio::time::timeout(establishment_timeout, async {
                if let Some(control) = attempts.as_deref_mut() { control.dispatch_started()?; }
                Ok::<_, ControlledProviderError>(build().send().await)
            }) => match response {
                Ok(Ok(Ok(response))) => Ok(response),
                Ok(Ok(Err(error))) => Err(AttemptError::Network {
                    connection_reset: is_connection_reset(&error),
                }),
                Ok(Err(error)) => return Err(error),
                Err(_) => Err(AttemptError::Timeout),
            },
        };
        match response {
            Ok(response) if response.status().is_success() => {
                if let Some(control) = attempts.as_deref_mut() {
                    control.hold_successful_response()?;
                }
                return Ok(response);
            }
            Ok(response) if retryable(response.status()) && attempt < delays.len() => {
                if let Some(control) = attempts.as_deref_mut() {
                    control
                        .settle_active(EffectOutcome::Unknown, None, false)
                        .await?;
                }
                let delay = parse_retry_after(response.headers())
                    .unwrap_or_else(|| jitter(delays[attempt]))
                    .min(MAX_BACKOFF);
                wait(delay, cancellation).await?;
            }
            Ok(response) => {
                if let Some(control) = attempts.as_deref_mut() {
                    control
                        .settle_active(EffectOutcome::Unknown, None, false)
                        .await?;
                }
                return Err(ProviderError::Request(describe(response.status())).into());
            }
            Err(AttemptError::Network {
                connection_reset: true,
            }) if attempt < delays.len() => {
                if let Some(control) = attempts.as_deref_mut() {
                    control
                        .settle_active(EffectOutcome::Unknown, None, false)
                        .await?;
                }
                let delay = jitter(delays[attempt]).min(MAX_BACKOFF);
                wait(delay, cancellation).await?;
            }
            Err(error) => {
                if let Some(control) = attempts.as_deref_mut() {
                    control
                        .settle_active(EffectOutcome::Unknown, None, false)
                        .await?;
                }
                if matches!(error, AttemptError::Timeout) {
                    return Err(ProviderError::Request(format!(
                        "provider request timed out while establishing a connection to {endpoint}"
                    ))
                    .into());
                }
                return Err(ProviderError::Request(format!(
                    "could not reach the provider at {endpoint} — check that it is running and the configured base_url is correct"
                )).into());
            }
        }
    }
    Err(ProviderError::Request(format!(
        "could not reach the provider at {endpoint} — check that it is running and the configured base_url is correct"
    )).into())
}

pub(super) fn provider_error(error: ControlledProviderError) -> ProviderError {
    match error {
        ControlledProviderError::Provider(error) => error,
        _ => ProviderError::Request("controlled provider call failed".into()),
    }
}

async fn wait(delay: Duration, cancellation: &CancellationToken) -> Result<(), ProviderError> {
    tokio::select! {
        _ = cancellation.cancelled() => Err(ProviderError::Cancelled),
        _ = tokio::time::sleep(delay) => Ok(()),
    }
}

/// Whether a rejected status is worth retrying: a rate limit (429) or a
/// server-side failure (5xx, which includes Anthropic's 529 overloaded
/// response). Every other 4xx is the request's own fault — a wrong model,
/// key, or body — and re-sending it would fail identically.
fn retryable(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

/// Whether a transport failure is a connection reset/abort worth retrying.
/// A refused connection, DNS failure, or timeout is not: the first two would
/// fail the same way again, and a timeout already spent its establishment
/// budget. The reset is looked for anywhere in the error's source chain
/// because `reqwest` wraps the `std::io::Error` at different depths.
fn is_connection_reset(error: &reqwest::Error) -> bool {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(current) = source {
        if let Some(io) = current.downcast_ref::<std::io::Error>()
            && matches!(
                io.kind(),
                std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::UnexpectedEof
            )
        {
            return true;
        }
        source = current.source();
    }
    false
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
#[path = "http/tests.rs"]
mod tests;
