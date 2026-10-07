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

use saya_agent::CancellationToken;
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

/// One download call: everything the entry point needs, bundled so the
/// argument list stays under the lint cap and the turn's token rides beside
/// the bounds it shares.
pub struct DownloadCall<'a> {
    /// The policy that judges the URL and every redirect target.
    pub policy: &'a FetchPolicy,
    /// The wire the request rides.
    pub transport: &'a dyn FetchTransport,
    /// The workspace the download lands in.
    pub workspace: &'a Workspace,
    /// The workspace-relative destination.
    pub destination: &'a str,
    /// The destination precondition, supplied per call: `None` is
    /// create-if-absent — an existing destination refuses before any
    /// network I/O and again at publication; `Some(sha256)` replaces only a
    /// destination whose current bytes hash to it, judged at the rename. A
    /// partial's sidecar proves only the partial's own URL, bytes, and
    /// digest — it never grants replacement authority.
    pub expected_destination_sha256: Option<&'a str>,
    /// The URL to download.
    pub url: &'a str,
    /// The per-attempt and per-file bounds.
    pub limits: DownloadLimits,
    /// The run's shared download wallet.
    pub budget: &'a DownloadBudget,
    /// The turn's cancellation token.
    pub cancellation: &'a CancellationToken,
}

/// One bounded, resumable download under the turn's cancellation token.
/// `destination` is a workspace-relative path (contained exactly like
/// `workspace_write`); `url` is judged by the policy here, before anything
/// is fetched. A recorded, digest-verified partial for the same URL is
/// resumed with a ranged GET; anything else is a fresh download. No disk
/// state is created until a body actually streams: a refused hop or a bad
/// status writes nothing.
///
/// Cancellation rides beside every bound: a cancelled turn is refused at
/// admission, at the top of every hop, inside every transport wait, and at
/// the last point before publication — nothing is ever published after a
/// cancellation was observed, and any partial in progress is persisted with
/// exactly the bytes it holds ([`DownloadError::Cancelled`]).
pub async fn http_download_cancellable(
    call: DownloadCall<'_>,
) -> Result<DownloadOutcome, DownloadError> {
    // Admission first: a cancelled turn starts nothing at all — not even a
    // sidecar read.
    if call.cancellation.is_cancelled() {
        return Err(DownloadError::Cancelled);
    }
    // Containment first: the destination argument is judged by the same
    // path discipline as any workspace write — before anything is scanned
    // or created.
    crate::workspace::contain::validate_argument(call.destination)?;
    let first = call.policy.allow_url(call.url)?;
    // The sidecar records the URL the run asked for, so a resume is only
    // attempted against the same request — not whatever a redirect chain
    // last landed on.
    let requested = call.url.to_owned();
    let recorded = partial::load_meta(call.workspace, call.destination)?;
    // The resume plan is verified read-only, before the request: the digest
    // state carries into the session, and a refused hop touches nothing.
    let mut plan = match recorded.as_ref().filter(|meta| meta.url == call.url) {
        Some(meta) => resume::verify_resume(call.workspace, call.destination, meta)?,
        None => None,
    };
    // Fail-fast, best-effort — the publication guard stays authoritative.
    super::download_guard::refuse_create_over_existing(
        call.workspace,
        call.destination,
        call.expected_destination_sha256,
    )?;
    let range_start = plan.as_ref().map(|plan| plan.meta.len);
    let mut url = first;
    let mut hops = 0usize;
    loop {
        // Top of every hop, before DNS: a cancelled turn consumes no further
        // hop, and a pre-cancelled one starts none.
        if call.cancellation.is_cancelled() {
            return Err(DownloadError::Cancelled);
        }
        let deadline = Instant::now() + call.limits.request_timeout;
        let host = url.host().unwrap_or_default().to_owned();
        let addresses = under_deadline(
            call.limits.request_timeout,
            deadline,
            call.cancellation,
            call.transport.resolve(&host),
        )
        .await?;
        // Resolve-then-deny over every returned address, before anything
        // connects — the same seam discipline as `http_fetch`.
        for address in addresses {
            refuses_address(address)?;
        }
        let response = under_deadline(
            call.limits.request_timeout,
            deadline,
            call.cancellation,
            call.transport.get(FetchRequest {
                url: url.clone(),
                range_start,
            }),
        )
        .await?;
        if matches!(response.status, 301 | 302 | 303 | 307 | 308) {
            match response.location.as_deref() {
                Some(location) => {
                    hops += 1;
                    if hops > call.limits.max_redirect_hops {
                        return Err(DownloadError::TooManyRedirects {
                            limit: call.limits.max_redirect_hops,
                        });
                    }
                    url = call.policy.allow_redirect(&url, location)?;
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
                let (path, file) = resume::reopen(call.workspace, call.destination, &plan.meta)?;
                Session {
                    path,
                    file,
                    hasher: plan.hasher,
                    total: plan.meta.len,
                }
            }
            None => session::fresh(call.workspace, call.destination, &requested)?,
        };
        session::stream_body(
            &mut session,
            response,
            call.workspace,
            call.destination,
            &requested,
            session::Bounds {
                limits: call.limits,
                budget: call.budget,
                cancellation: call.cancellation,
            },
        )
        .await?;
        // The last refusal point before publication: the promotion is one
        // atomic rename and the code between here and it is synchronous, so
        // a cancellation observed now publishes nothing — the complete
        // partial and its sidecar stay for an explicit retry. After the
        // promotion returns, the download is committed and reported as the
        // success it is, whatever the token says by then.
        if call.cancellation.is_cancelled() {
            let _ = session.persist(call.workspace, call.destination, &requested);
            return Err(DownloadError::Cancelled);
        }
        let digest = partial::hex(session.hasher.finalize().as_slice());
        partial::promote(
            call.workspace,
            call.destination,
            call.expected_destination_sha256,
        )?;
        return Ok(DownloadOutcome {
            destination: call.destination.to_owned(),
            bytes: session.total,
            sha256: digest,
        });
    }
}

/// One bounded, resumable download under a fresh, never-cancelled token —
/// the public entry point for callers with no turn token to honor.
pub async fn http_download(
    policy: &FetchPolicy,
    transport: &dyn FetchTransport,
    workspace: &Workspace,
    destination: &str,
    url: &str,
    limits: DownloadLimits,
    budget: &DownloadBudget,
) -> Result<DownloadOutcome, DownloadError> {
    let cancellation = CancellationToken::new();
    http_download_cancellable(DownloadCall {
        policy,
        transport,
        workspace,
        destination,
        // Create-only: this legacy wrapper carries no destination digest.
        expected_destination_sha256: None,
        url,
        limits,
        budget,
        cancellation: &cancellation,
    })
    .await
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

/// One transport operation cut off at the request deadline or the turn's
/// token: a transport that never yields is dropped at the bound, and a
/// cancelled turn stops waiting at once. The deadline future is polled
/// first; the hop-top and pre-publication checks convert late arrivals.
pub(super) async fn under_deadline<T>(
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
