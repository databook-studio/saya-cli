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
//! [`ContextBlock`] — the labelled, delimited lane whose renderer escapes any
//! body that tries to forge the closing sentinel — never in the system
//! prompt and never as bare tool-result text.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use saya_agent::ContextBlock;
use tokio::time::{Instant, timeout_at};

use super::limits::FetchLimits;
use super::policy::{FetchPolicy, FetchRefusal, refuses_address};
use super::transport::{FetchBody, FetchRequest, FetchTransport, FetchTransportError};

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

/// What a successful fetch hands back: the final URL after any judged
/// redirects, and the content as a [`ContextBlock`] — the only lane on which
/// it may reach the model.
#[derive(Debug, Clone)]
pub struct FetchOutcome {
    /// The final URL the body was fetched from.
    pub url: String,
    /// The body as untrusted context: the renderer escapes it, the model
    /// reads it as data.
    pub block: ContextBlock,
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

    /// One bounded fetch. Every hop passes the policy, every resolved
    /// address passes `refuses_address`, and the body is read under the
    /// bounds; anything else is a typed refusal or failure.
    pub async fn fetch(&self, url: &str) -> Result<FetchOutcome, FetchToolError> {
        let budget = self.limits.time_budget;
        let deadline = Instant::now() + budget;
        let mut url = self.policy.allow_url(url)?;
        let mut redirects = 0usize;
        loop {
            let host = url.host().unwrap_or_default().to_owned();
            let addresses = self
                .under_deadline(deadline, self.transport.resolve(&host))
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
                    self.transport.get(FetchRequest { url: url.clone() }),
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
                    let body = self.read_body(response.body, deadline).await?;
                    return Ok(FetchOutcome {
                        url: url.as_str().to_owned(),
                        block: ContextBlock {
                            label: format!("http-fetch {host}"),
                            body,
                            truncated: false,
                        },
                    });
                }
            }
        }
    }

    /// Reads the body chunk by chunk under the byte bound and the shared
    /// deadline. An overrun is a typed error; a success is always the whole
    /// body, so `truncated` is false by construction.
    async fn read_body(
        &self,
        mut body: Box<dyn FetchBody>,
        deadline: Instant,
    ) -> Result<String, FetchToolError> {
        let mut bytes: Vec<u8> = Vec::new();
        while let Some(chunk) = self.under_deadline(deadline, body.next_chunk()).await? {
            if bytes.len() + chunk.len() > self.limits.max_total_bytes {
                return Err(FetchToolError::BodyTooLarge {
                    limit: self.limits.max_total_bytes,
                });
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// One transport operation cut off at the shared deadline: a transport
    /// that never yields is dropped at the bound, not waited on forever.
    async fn under_deadline<T>(
        &self,
        deadline: Instant,
        future: impl Future<Output = Result<T, FetchTransportError>> + Send,
    ) -> Result<T, FetchToolError>
    where
        T: Send,
    {
        match timeout_at(deadline, future).await {
            Ok(result) => result.map_err(FetchToolError::Transport),
            Err(_) => Err(FetchToolError::DeadlineExceeded {
                budget: self.limits.time_budget,
            }),
        }
    }
}
