//! `http_download`: the bounded, resumable download of a policy-judged URL
//! into the run workspace.
//!
//! The order of every download is the fetch order, fail-closed: the policy
//! judges the URL (first hop) and every redirect target; the transport
//! resolves the accepted host and any refused address refuses the request
//! before anything connects; then the body streams — chunk by chunk, never
//! buffered whole — into the workspace under three bounds, each tripping as
//! a typed error before the byte that would exceed it is written:
//!
//! * the per-file byte bound,
//! * the run's shared [`DownloadBudget`] — tripped **fail-safe: paused, not
//!   overrun** (the engine maps it to `PauseReason::BudgetExhausted`),
//! * the per-request wall clock, which bounds one attempt; an interrupted
//!   attempt leaves a resumable partial ([`super::partial`]), so a long
//!   transfer is a sequence of bounded, resumable attempts.
//!
//! The destination path is contained exactly like `workspace_write`: the
//! final file lands by atomic rename, 0600, never executable.

use std::time::Duration;

use sha2::Digest;
use tokio::time::{Instant, timeout_at};

use super::budget::DownloadBudget;
use super::download_error::{DownloadError, DownloadOutcome};
use super::download_limits::DownloadLimits;
use super::partial;
use super::policy::{FetchPolicy, refuses_address};
use super::resume;
use super::session::{self, Session};
use super::transport::{FetchRequest, FetchTransport, FetchTransportError, WireResponse};
use crate::workspace::Workspace;

/// One bounded, resumable download. `destination` is a workspace-relative
/// path (contained exactly like `workspace_write`); `url` is judged by the
/// policy here, before anything is fetched. A recorded, digest-verified
/// partial for the same URL is resumed with a ranged GET; anything else is a
/// fresh download. No disk state is created until a body actually streams:
/// a refused hop or a bad status writes nothing.
pub async fn http_download(
    policy: &FetchPolicy,
    transport: &dyn FetchTransport,
    workspace: &Workspace,
    destination: &str,
    url: &str,
    limits: DownloadLimits,
    budget: &DownloadBudget,
) -> Result<DownloadOutcome, DownloadError> {
    // Containment first: the destination argument is judged by the same
    // path discipline as any workspace write — before anything is scanned
    // or created.
    crate::workspace::contain::validate_argument(destination)?;
    let first = policy.allow_url(url)?;
    // The sidecar records the URL the run asked for, so a resume is only
    // attempted against the same request — not whatever a redirect chain
    // last landed on.
    let requested = url.to_owned();
    let recorded = partial::load_meta(workspace, destination)?;
    // The resume plan is verified read-only, before the request: the digest
    // state carries into the session, and a refused hop touches nothing.
    let mut plan = match recorded.as_ref().filter(|meta| meta.url == url) {
        Some(meta) => resume::verify_resume(workspace, destination, meta)?,
        None => None,
    };
    let range_start = plan.as_ref().map(|plan| plan.meta.len);
    let mut url = first;
    let mut hops = 0usize;
    loop {
        let deadline = Instant::now() + limits.request_timeout;
        let host = url.host().unwrap_or_default().to_owned();
        let addresses =
            under_deadline(limits.request_timeout, deadline, transport.resolve(&host)).await?;
        // Resolve-then-deny over every returned address, before anything
        // connects — the same seam discipline as `http_fetch`.
        for address in addresses {
            refuses_address(address)?;
        }
        let response = under_deadline(
            limits.request_timeout,
            deadline,
            transport.get(FetchRequest {
                url: url.clone(),
                range_start,
            }),
        )
        .await?;
        if matches!(response.status, 301 | 302 | 303 | 307 | 308) {
            match response.location.as_deref() {
                Some(location) => {
                    hops += 1;
                    if hops > limits.max_redirect_hops {
                        return Err(DownloadError::TooManyRedirects {
                            limit: limits.max_redirect_hops,
                        });
                    }
                    url = policy.allow_redirect(&url, location)?;
                    continue;
                }
                None => {
                    return Err(DownloadError::UnsuccessfulStatus {
                        url: url.as_str().to_owned(),
                        status: response.status,
                    });
                }
            }
        }
        if !(200..300).contains(&response.status) {
            return Err(DownloadError::UnsuccessfulStatus {
                url: url.as_str().to_owned(),
                status: response.status,
            });
        }
        if response.status == 206 {
            verify_range(url.as_str(), &response, range_start)?;
        }
        if response.status == 200 && range_start.is_some() {
            // The server ignored the range and served the whole resource;
            // appending a whole body onto a partial would corrupt it. The
            // only correct action is to restart from zero.
            plan = None;
        }
        // The session exists only once a body actually streams: the partial
        // and its sidecar appear with it, and a refusal above wrote nothing.
        let mut session = match plan.take() {
            Some(plan) => {
                let (path, file) = resume::reopen(workspace, destination, &plan.meta)?;
                Session {
                    path,
                    file,
                    hasher: plan.hasher,
                    total: plan.meta.len,
                }
            }
            None => session::fresh(workspace, destination, &requested)?,
        };
        session::stream_body(
            &mut session,
            response,
            workspace,
            destination,
            &requested,
            limits,
            budget,
        )
        .await?;
        let digest = partial::hex(session.hasher.finalize().as_slice());
        partial::promote(workspace, destination)?;
        return Ok(DownloadOutcome {
            destination: destination.to_owned(),
            bytes: session.total,
            sha256: digest,
        });
    }
}

/// Verifies a 206 against the offset the resume asked for.
fn verify_range(
    url: &str,
    response: &WireResponse,
    expected: Option<u64>,
) -> Result<(), DownloadError> {
    let expected = expected.unwrap_or_default();
    let mismatch = |served: String| DownloadError::RangeMismatch {
        url: url.to_owned(),
        expected,
        served,
    };
    let served = response
        .content_range
        .as_deref()
        .ok_or_else(|| mismatch("(absent)".to_owned()))?;
    let served_start = served
        .strip_prefix("bytes ")
        .and_then(|rest| rest.split('-').next())
        .and_then(|offset| offset.parse::<u64>().ok());
    match served_start {
        Some(start) if start == expected => Ok(()),
        _ => Err(mismatch(served.to_owned())),
    }
}

/// One transport operation cut off at the request deadline.
pub(super) async fn under_deadline<T>(
    budget: Duration,
    deadline: Instant,
    future: impl std::future::Future<Output = Result<T, FetchTransportError>> + Send,
) -> Result<T, DownloadError>
where
    T: Send,
{
    match timeout_at(deadline, future).await {
        Ok(result) => result.map_err(DownloadError::Transport),
        Err(_) => Err(DownloadError::DeadlineExceeded { budget }),
    }
}
