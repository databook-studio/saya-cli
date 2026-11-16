//! The `http_fetch` tool: a bounded GET through the egress policy, delivered
//! to the model only as an untrusted [`ContextBlock`].
//!
//! The order of every fetch is fixed and fail-closed: the policy judges the
//! URL (first hop) and every redirect target (per hop); the transport then
//! resolves the accepted host and the tool refuses on any returned address
//! the policy refuses — resolve-then-deny, before anything connects; only
//! then is the request sent, and its body read under the explicit bounds of
//! [`FetchLimits`]. Redirects are never followed by the transport; each hop
//! is a fresh policy decision.
//!
//! Fetched bytes are untrusted. They reach the model only as a
//! [`ContextBlock`] — the labelled, delimited block whose renderer escapes any
//! body that tries to forge the closing sentinel — never in the system
//! prompt and never as bare tool-result text.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use saya_agent::{CancellationToken, ContextBlock};
use tokio::time::{Instant, timeout_at};

use super::limits::FetchLimits;
use super::policy::{FetchPolicy, FetchRefusal, refuses_address};
use super::transport::{
    FetchBody, FetchHeaders, FetchRequest, FetchTransport, FetchTransportError,
};
use crate::runner::OutputRing;

/// Why a fetch failed. Every bound overrun is typed — a truncated body is
/// never passed off as the whole one.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FetchToolError {
    /// The URL or a redirect target was refused by the policy, or a resolved
    /// address was refused before connecting.
    #[error("fetch refused: {0}")]
    Refused(#[from] FetchRefusal),

    /// The body exceeded `max_total_bytes`.
    #[error("fetch body exceeded {limit} bytes")]
    BodyTooLarge {
        /// The byte bound that tripped.
        limit: usize,
    },

    /// DNS, a hop, or the body read did not finish inside the time budget.
    #[error("fetch exceeded its {budget:.0?} wall-clock budget")]
    DeadlineExceeded {
        /// The time budget that expired.
        budget: Duration,
    },

    /// The turn was cancelled while the fetch waited.
    #[error("fetch cancelled")]
    Cancelled,

    /// More redirects were followed than `max_redirect_hops` allows.
    #[error("fetch exceeded {limit} redirects")]
    TooManyRedirects {
        /// The redirect bound that tripped.
        limit: usize,
    },

    /// The final response was not a success status.
    #[error("fetch of {url} returned status {status}")]
    UnsuccessfulStatus {
        /// The URL the request was made against.
        url: String,
        /// The response status.
        status: u16,
    },

    /// The transport failed — an unresolvable name, a broken connection.
    #[error("fetch transport failed: {0}")]
    Transport(FetchTransportError),
}

/// How a fetch's body is bounded once a success response arrives. Stated by
/// the caller, because the two readers differ in what a bound *means*: one
/// refuses to pass off a short body as the whole one, the other hands back
/// both ends of what it could not fit and says how much it dropped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BodyBound {
    /// The whole body or a typed [`FetchToolError::BodyTooLarge`] — the
    /// `http_fetch` rule, whose destination is one bounded context turn.
    Whole,
    /// Head and tail inside `cap` bytes with the elision marker between them
    /// — the `web_fetch` rule. Memory stays bounded by `cap` whatever
    /// the response streams, and the outcome reports exactly what was
    /// dropped, so a long page is never silently shortened.
    HeadTail { cap: usize },
}

/// What a successful fetch hands back: the final URL after any judged
/// redirects, the response's own status and content type, and the content as
/// a [`ContextBlock`] — the only path on which it may reach the model.
#[derive(Debug, Clone)]
pub struct FetchOutcome {
    /// The final URL the body was fetched from.
    pub url: String,
    /// The final response's status, always a success status.
    pub status: u16,
    /// The final response's `Content-Type`, when the server sent one.
    pub content_type: Option<String>,
    /// The body as untrusted context: the renderer escapes it, the model
    /// reads it as data.
    pub block: ContextBlock,
    /// Whether the body's bound elided bytes (never true for
    /// [`BodyBound::Whole`], which fails instead).
    pub truncated: bool,
    /// The bytes the bound elided. Zero when nothing was.
    pub dropped_bytes: u64,
    /// The body's real total byte count, retained or not.
    pub total_bytes: u64,
}

/// Bounded GETs through the run's destination policy and a [`FetchTransport`].
#[derive(Clone)]
pub struct HttpFetchTool {
    policy: FetchPolicy,
    transport: Arc<dyn FetchTransport>,
    limits: FetchLimits,
}

impl HttpFetchTool {
    /// Builds the tool from the run's approved fetch policy, the wire it
    /// drives, and its bounds.
    pub fn new(
        policy: FetchPolicy,
        transport: Arc<dyn FetchTransport>,
        limits: FetchLimits,
    ) -> Self {
        Self {
            policy,
            transport,
            limits,
        }
    }

    /// One bounded fetch under a fresh, never-cancelled token — the public
    /// entry point for callers with no turn token to honor.
    pub async fn fetch(&self, url: &str) -> Result<FetchOutcome, FetchToolError> {
        self.fetch_cancellable(url, &CancellationToken::new()).await
    }

    /// The same bounded fetch under the turn's cancellation token: every
    /// transport wait races the token, a cancelled turn never begins another
    /// hop, and a fetch that completes only after cancellation returns
    /// cancelled, never a result. Every byte and time bound is unchanged.
    pub async fn fetch_cancellable(
        &self,
        url: &str,
        cancellation: &CancellationToken,
    ) -> Result<FetchOutcome, FetchToolError> {
        self.fetch_bounded_cancellable(url, BodyBound::Whole, cancellation)
            .await
    }

    /// The same fetch with the body's bound stated by the caller: the whole
    /// body or a typed error ([`BodyBound::Whole`], which
    /// [`fetch_cancellable`](Self::fetch_cancellable) uses), or head and tail
    /// with the elision marker ([`BodyBound::HeadTail`]). The policy, the
    /// resolve-then-deny check, the per-hop redirect judgement, the wall
    /// clock and the redirect bound are identical either way — only what a
    /// body over its bound *means* differs.
    pub async fn fetch_bounded_cancellable(
        &self,
        url: &str,
        bound: BodyBound,
        cancellation: &CancellationToken,
    ) -> Result<FetchOutcome, FetchToolError> {
        self.fetch_bounded_with_headers(url, bound, &FetchHeaders::default(), cancellation)
            .await
    }

    /// The same fetch carrying the caller's own headers on its **first**
    /// request: what a tool that authenticates to a configured endpoint adds
    /// to the request it built. The headers are cleared before any redirect,
    /// so a credential never crosses into a hop the operator did not state;
    /// every other rule — the policy, resolve-then-deny, the per-hop
    /// judgement, the wall clock, the redirect bound — is unchanged.
    pub async fn fetch_bounded_with_headers(
        &self,
        url: &str,
        bound: BodyBound,
        headers: &FetchHeaders,
        cancellation: &CancellationToken,
    ) -> Result<FetchOutcome, FetchToolError> {
        let budget = self.limits.time_budget;
        let deadline = Instant::now() + budget;
        let mut url = self.policy.allow_url(url)?;
        let mut redirects = 0usize;
        // Taken by the first hop and left empty for every hop after it.
        let mut hop_headers = headers.clone();
        loop {
            // Top of every hop, before DNS: a cancelled turn consumes no
            // further hop, and a pre-cancelled one starts none.
            if cancellation.is_cancelled() {
                return Err(FetchToolError::Cancelled);
            }
            let host = url.host().unwrap_or_default().to_owned();
            let addresses = self
                .under_deadline(deadline, cancellation, self.transport.resolve(&host))
                .await?;
            // The policy's documented residual, closed here: it judges the
            // name, not what it resolves to. Every returned address must be
            // refused-or-allowed before anything connects — never connect
            // first and check after.
            for address in addresses {
                refuses_address(address)?;
            }
            let response = self
                .under_deadline(
                    deadline,
                    cancellation,
                    self.transport.get(FetchRequest {
                        url: url.clone(),
                        range_start: None,
                        headers: std::mem::take(&mut hop_headers),
                    }),
                )
                .await?;
            let redirect = matches!(response.status, 301 | 302 | 303 | 307 | 308);
            match (redirect, response.location.as_deref()) {
                // The hop's body is never read: the response is dropped here,
                // and the target is judged as a fresh URL.
                (true, Some(location)) => {
                    redirects += 1;
                    if redirects > self.limits.max_redirect_hops {
                        return Err(FetchToolError::TooManyRedirects {
                            limit: self.limits.max_redirect_hops,
                        });
                    }
                    url = self.policy.allow_redirect(&url, location)?;
                }
                _ => {
                    if !(200..300).contains(&response.status) {
                        return Err(FetchToolError::UnsuccessfulStatus {
                            url: url.as_str().to_owned(),
                            status: response.status,
                        });
                    }
                    let status = response.status;
                    let content_type = response.content_type;
                    let (body, truncated, dropped_bytes, total_bytes) = self
                        .read_body(response.body, bound, deadline, cancellation)
                        .await?;
                    // A fetch that completed only after cancellation returns
                    // cancelled, never a result.
                    if cancellation.is_cancelled() {
                        return Err(FetchToolError::Cancelled);
                    }
                    return Ok(FetchOutcome {
                        url: url.as_str().to_owned(),
                        status,
                        content_type,
                        block: ContextBlock {
                            label: format!("http-fetch {host}"),
                            body,
                            truncated,
                        },
                        truncated,
                        dropped_bytes,
                        total_bytes,
                    });
                }
            }
        }
    }

    /// Reads the body block by block under the bound and the shared deadline,
    /// returning `(text, truncated, dropped_bytes, total_bytes)`. Under
    /// [`BodyBound::Whole`] an overrun is a typed error and a success is
    /// always the whole body, so `truncated` is false by construction; under
    /// [`BodyBound::HeadTail`] the bytes flow through the runner's own
    /// head-and-tail ring, so memory stays inside the cap and the report is
    /// the ring's honest one.
    async fn read_body(
        &self,
        mut body: Box<dyn FetchBody>,
        bound: BodyBound,
        deadline: Instant,
        cancellation: &CancellationToken,
    ) -> Result<(String, bool, u64, u64), FetchToolError> {
        let ring = match bound {
            BodyBound::Whole => None,
            BodyBound::HeadTail { cap } => Some(OutputRing::new(cap)),
        };
        let mut bytes: Vec<u8> = Vec::new();
        let mut total: u64 = 0;
        while let Some(chunk) = self
            .under_deadline(deadline, cancellation, body.next_chunk())
            .await?
        {
            total += chunk.len() as u64;
            match &ring {
                Some(ring) => ring.push(&chunk),
                None => {
                    if bytes.len() + chunk.len() > self.limits.max_total_bytes {
                        return Err(FetchToolError::BodyTooLarge {
                            limit: self.limits.max_total_bytes,
                        });
                    }
                    bytes.extend_from_slice(&chunk);
                }
            }
        }
        Ok(match ring {
            Some(ring) => ring.snapshot(),
            None => (
                String::from_utf8_lossy(&bytes).into_owned(),
                false,
                0,
                total,
            ),
        })
    }

    /// One transport operation cut off at the shared deadline or the turn's
    /// token: a transport that never yields is dropped at the bound, and a
    /// cancelled turn stops waiting at once. The deadline future is polled
    /// first; the hop-top and pre-success checks convert late arrivals.
    async fn under_deadline<T>(
        &self,
        deadline: Instant,
        cancellation: &CancellationToken,
        future: impl Future<Output = Result<T, FetchTransportError>> + Send,
    ) -> Result<T, FetchToolError>
    where
        T: Send,
    {
        tokio::select! {
            biased;
            result = timeout_at(deadline, future) => match result {
                Ok(result) => result.map_err(FetchToolError::Transport),
                Err(_) => Err(FetchToolError::DeadlineExceeded {
                    budget: self.limits.time_budget,
                }),
            },
            _ = cancellation.cancelled() => Err(FetchToolError::Cancelled),
        }
    }
}
