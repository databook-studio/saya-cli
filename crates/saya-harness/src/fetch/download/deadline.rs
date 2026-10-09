//! The transport bounds of one hop: the per-attempt wall clock and the
//! turn's cancellation token, raced in one place.

use std::time::Duration;

use saya_agent::CancellationToken;
use tokio::time::{Instant, timeout_at};

use super::super::download_error::DownloadError;
use super::super::transport::FetchTransportError;

/// One transport operation cut off at the request deadline or the turn's
/// token: a transport that never yields is dropped at the bound, and a
/// cancelled turn stops waiting at once. The deadline future is polled
/// first; the hop-top and pre-publication checks convert late arrivals.
pub(in crate::fetch) async fn under_deadline<T>(
    budget: Duration,
    deadline: Instant,
    cancellation: &CancellationToken,
    future: impl std::future::Future<Output = Result<T, FetchTransportError>> + Send,
) -> Result<T, DownloadError>
where
    T: Send,
{
    tokio::select! {
        biased;
        result = timeout_at(deadline, future) => match result {
            Ok(result) => result.map_err(DownloadError::Transport),
            Err(_) => Err(DownloadError::DeadlineExceeded { budget }),
        },
        _ = cancellation.cancelled() => Err(DownloadError::Cancelled),
    }
}
