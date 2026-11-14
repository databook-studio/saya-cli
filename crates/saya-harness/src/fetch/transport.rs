//! The network seam behind the fetch tool: name resolution and one GET,
//! behind a trait so the policy-judged decision can be tested hermetically
//! and so the production HTTP client arrives as an implementation of this
//! seam rather than as a dependency of the decision logic.
//!
//! The tool drives this seam only with [`FetchUrl`]s its policy already
//! judged, and it — never the transport — decides redirects and address
//! refusals. A transport must therefore surface a redirect as a status and
//! header instead of following it, and must not resolve-and-connect on its
//! own authority.

use std::fmt;
use std::net::IpAddr;

use async_trait::async_trait;
use thiserror::Error;

use super::policy::FetchUrl;

/// The most caller-stated headers one request may carry. A header set is built
/// from trusted configuration, never from model arguments, so this is a
/// mistake-catcher rather than a hostile-input bound — a bound all the same.
pub const MAX_REQUEST_HEADERS: usize = 4;

/// The most bytes one header name or value may carry.
pub const MAX_HEADER_BYTES: usize = 1024;

/// A small, caller-stated header set for one request: what a tool that must
/// authenticate to a configured endpoint adds to the request it built. It is
/// never model input — the model states a URL and a query, and the tool turns
/// the trusted configuration into headers.
///
/// Deliberately not `Debug`-derived: a value here is a credential, and a
/// derived formatter would put it in any log that printed the set. The manual
/// form renders the *names*, which is what a diagnostic needs.
#[derive(Clone, Default)]
pub struct FetchHeaders(Vec<(String, String)>);

impl FetchHeaders {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one header, refusing a name that is not a lowercase HTTP token, a
    /// value that is not visible ASCII, either one oversized, and a set that
    /// already holds [`MAX_REQUEST_HEADERS`].
    pub fn push(&mut self, name: &str, value: &str) -> Result<(), FetchHeaderError> {
        if self.0.len() >= MAX_REQUEST_HEADERS {
            return Err(FetchHeaderError::TooMany);
        }
        if name.is_empty()
            || name.len() > MAX_HEADER_BYTES
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(FetchHeaderError::InvalidName);
        }
        if value.is_empty()
            || value.len() > MAX_HEADER_BYTES
            || !value.bytes().all(|byte| (0x20..0x7f).contains(&byte))
        {
            return Err(FetchHeaderError::InvalidValue);
        }
        self.0.push((name.to_owned(), value.to_owned()));
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The headers, name and value, in the order they were added.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }
}

impl fmt::Debug for FetchHeaders {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FetchHeaders")
            .field(
                "names",
                &self
                    .0
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// Why a header set was refused. Every variant is a caller mistake, refused
/// typed rather than normalized into a different request.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum FetchHeaderError {
    /// The set already holds [`MAX_REQUEST_HEADERS`].
    #[error("a request carries at most {MAX_REQUEST_HEADERS} headers")]
    TooMany,

    /// The name is empty, oversized, or not a lowercase HTTP token.
    #[error("a header name must be a lowercase token of 1 to {MAX_HEADER_BYTES} bytes")]
    InvalidName,

    /// The value is empty, oversized, or not visible ASCII.
    #[error("a header value must be 1 to {MAX_HEADER_BYTES} bytes of visible ASCII")]
    InvalidValue,
}

/// One policy-judged request. The URL is a [`FetchUrl`], constructible only
/// through a [`FetchPolicy`](super::policy::FetchPolicy), so no caller of the
/// transport can ask the wire for a URL the policy did not accept.
pub struct FetchRequest {
    /// The accepted URL, as the fetcher will use it.
    pub url: FetchUrl,
    /// The resume offset for a ranged GET: when set,
    /// the transport sends `Range: bytes=<start>-` so a partial can be
    /// continued instead of restarted. `None` is an ordinary full GET.
    pub range_start: Option<u64>,
    /// Headers the caller states for this one request. The fetch loop sends
    /// them on the first hop and clears them before any redirect: a redirect
    /// is a destination the operator never stated, so a credential that rode
    /// it would be a credential sent to a host nobody chose.
    pub headers: FetchHeaders,
}

/// The head of one response, with the body left streaming behind a pull
/// trait so the tool can bound what it actually reads.
pub struct WireResponse {
    /// The response status code.
    pub status: u16,
    /// The `Location` header value when the server sent one. Redirects are
    /// surfaced here, never followed.
    pub location: Option<String>,
    /// The `Content-Range` header value when the server sent one (a 206 to
    /// a ranged request). The downloader reads the served offset from it
    /// and refuses a response that does not match the offset it asked for.
    pub content_range: Option<String>,
    /// The `Content-Type` header value when the server sent one. A reader
    /// that renders the body for the model reads it to decide *how* to
    /// render: an HTML page becomes text, anything else is passed through.
    pub content_type: Option<String>,
    /// The response body, block by block. `None` from
    /// [`FetchBody::next_chunk`] ends it.
    pub body: Box<dyn FetchBody>,
}

/// The close- or length-delimited body of one response, pulled block by
/// block.
#[async_trait]
pub trait FetchBody: Send {
    /// The next block of the body, or `None` at its end. The tool applies
    /// its byte and time bounds around each pull.
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, FetchTransportError>;
}

/// The wire: resolve a host the policy accepted, then GET a URL the policy
/// accepted. Redirects and refusals are decisions the tool makes; a
/// transport that followed redirects itself would fetch unjudged URLs.
#[async_trait]
pub trait FetchTransport: Send + Sync {
    /// Every address `host` resolves to, in resolution order. The tool
    /// refuses the whole request if any one of them is refused
    /// (resolve-then-deny over all returned addresses), before connecting.
    async fn resolve(&self, host: &str) -> Result<Vec<IpAddr>, FetchTransportError>;

    /// One GET of a policy-judged URL. A redirect response comes back as a
    /// status and `Location` for the tool to judge; it is never followed
    /// here.
    async fn get(&self, request: FetchRequest) -> Result<WireResponse, FetchTransportError>;
}

/// Why a transport operation failed. These are transport failures —
/// unresolvable names, broken connections — distinct from the policy's
/// refusals, which are decisions.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FetchTransportError {
    /// The host does not resolve. Never guessed at.
    #[error("DNS resolution failed for {host}")]
    Resolve {
        /// The host that did not resolve.
        host: String,
    },

    /// Connecting, writing, or reading failed mid-request.
    #[error("request to {url} failed: {detail}")]
    Request {
        /// The URL the request was for.
        url: String,
        /// The underlying failure's own text.
        detail: String,
    },
}
