use super::*;
use crate::{
    EffectAdmission, EffectAdmissionError, EffectCallKind, EffectOwnerFailure, EffectPermit,
    EffectSettlement, PhysicalAttempt, ToolEffect,
};
use async_trait::async_trait;
use std::{
    num::NonZeroU64,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

// --- What the transport retries -------------------------------------------
//
// Only a rate limit (429), a server failure (5xx, including Anthropic's 529
// overloaded response), or a reset connection is retried. Every other 4xx is
// the request's own fault and fails fast; a timeout already spent its
// establishment budget.

/// A throwaway HTTP server that answers each accepted connection with the next
/// scripted status (500 once the script runs out) and counts what it saw, so a
/// test can read the transport's physical send count.
async fn scripted_server(
    statuses: &[StatusCode],
) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&seen);
    let statuses = statuses.to_vec();
    let handle = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let index = counter.fetch_add(1, Ordering::SeqCst);
            let status = statuses
                .get(index)
                .copied()
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            let mut buf = [0_u8; 1024];
            let _ = socket.readable().await;
            let _ = socket.try_read(&mut buf);
            let response = format!(
                "HTTP/1.1 {code} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                code = status.as_u16(),
                reason = status.canonical_reason().unwrap_or("Status"),
            );
            let _ = socket.writable().await;
            let _ = socket.try_write(response.as_bytes());
        }
    });
    (format!("http://{addr}"), seen, handle)
}

fn one_retry_delay() -> Vec<Duration> {
    vec![Duration::from_millis(1)]
}

/// A schedule with room for a second retry, so a repeated non-retryable status
/// proves the status rule, not the schedule bound.
fn two_retry_delays() -> Vec<Duration> {
    vec![Duration::from_millis(1), Duration::from_millis(1)]
}

async fn send(url: &str, delays: &[Duration]) -> Result<Response, ProviderError> {
    let client = reqwest::Client::new();
    let cancellation = CancellationToken::new();
    send_stream(
        || client.get(url),
        RetryPolicy::new(delays),
        &cancellation,
        url,
        Duration::from_secs(5),
    )
    .await
}

#[tokio::test]
async fn a_429_is_retried_and_the_retry_continues() {
    let (url, seen, server) =
        scripted_server(&[StatusCode::TOO_MANY_REQUESTS, StatusCode::OK]).await;

    let response = send(&url, &one_retry_delay()).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(seen.load(Ordering::SeqCst), 2, "exactly one extra send");
    server.abort();
}

#[tokio::test]
async fn a_server_error_is_retried_and_the_retry_continues() {
    let (url, seen, server) =
        scripted_server(&[StatusCode::SERVICE_UNAVAILABLE, StatusCode::OK]).await;

    let response = send(&url, &one_retry_delay()).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(seen.load(Ordering::SeqCst), 2, "exactly one extra send");
    server.abort();
}

#[tokio::test]
async fn a_400_is_never_retried() {
    let (url, seen, server) = scripted_server(&[StatusCode::BAD_REQUEST]).await;

    let error = send(&url, &two_retry_delays()).await.unwrap_err();

    assert!(error.to_string().contains("400"), "{error:?}");
    assert_eq!(seen.load(Ordering::SeqCst), 1, "a 4xx is not retried");
    server.abort();
}

#[tokio::test]
async fn a_404_is_never_retried_even_after_a_successful_request() {
    let (url, seen, server) = scripted_server(&[StatusCode::OK, StatusCode::NOT_FOUND]).await;

    let first = send(&url, &two_retry_delays()).await.unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let error = send(&url, &two_retry_delays()).await.unwrap_err();

    assert!(error.to_string().contains("404"), "{error:?}");
    assert_eq!(
        seen.load(Ordering::SeqCst),
        2,
        "the second request was sent once and not retried"
    );
    server.abort();
}

#[derive(Default)]
pub(super) struct CountingAdmission {
    pub(super) admitted: AtomicUsize,
    pub(super) settled: AtomicUsize,
    pub(super) unknown_settlements: AtomicUsize,
}

pub(super) fn permit(id: u64) -> EffectPermit {
    EffectPermit::new(NonZeroU64::new(id).unwrap())
}

#[async_trait]
impl EffectAdmission for CountingAdmission {
    async fn admit_model_request(&self) -> Result<EffectPermit, EffectAdmissionError> {
        Ok(permit(1))
    }

    async fn admit_tool_batch(
        &self,
        _effects: &[ToolEffect],
    ) -> Result<Vec<EffectPermit>, EffectAdmissionError> {
        Ok(Vec::new())
    }

    async fn admit_physical_attempt(
        &self,
        _attempt: PhysicalAttempt,
    ) -> Result<EffectPermit, EffectAdmissionError> {
        let ordinal = self.admitted.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(permit(ordinal as u64))
    }

    async fn settle(
        &self,
        _permit: EffectPermit,
        settlement: EffectSettlement,
    ) -> Result<(), EffectOwnerFailure> {
        if matches!(
            settlement,
            EffectSettlement::PhysicalAttempt {
                outcome: crate::EffectOutcome::Unknown,
                ..
            }
        ) {
            self.unknown_settlements.fetch_add(1, Ordering::SeqCst);
        }
        self.settled.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// The retry is an ordinary send: it is admitted before dispatch, so the
/// physical-attempt accounting counts it against the same bound as any other
/// attempt.
#[tokio::test]
async fn the_retried_server_error_is_admitted_as_a_second_physical_attempt() {
    let (url, seen, server) = scripted_server(&[StatusCode::BAD_GATEWAY, StatusCode::OK]).await;
    let delays = one_retry_delay();
    let admission = CountingAdmission::default();
    let cancellation = CancellationToken::new();
    let mut control = PhysicalAttemptControl::new(
        &admission,
        &cancellation,
        permit(99),
        EffectCallKind::Answer,
        None,
    );
    let client = reqwest::Client::new();

    let response = send_stream_controlled(
        || client.get(&url),
        RetryPolicy::new(&delays),
        &cancellation,
        &url,
        Duration::from_secs(5),
        &mut control,
    )
    .await
    .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(admission.admitted.load(Ordering::SeqCst), 2);
    assert_eq!(seen.load(Ordering::SeqCst), 2);
    server.abort();
}
