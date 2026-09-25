//! The network seam behind the fetch tool: name resolution and one GET,
//! behind a trait so the policy-judged decision can be tested hermetically
//! and so the production HTTP client arrives as an implementation of this
//! seam (a later slice) rather than as a dependency of the decision logic.
//!
//! The tool drives this seam only with [`FetchUrl`]s its policy already
//! judged, and it — never the transport — decides redirects and address
//! refusals. A transport must therefore surface a redirect as a status and
//! header instead of following it, and must not resolve-and-connect on its
//! own authority.

use std::net::IpAddr;

use async_trait::async_trait;

use super::policy::FetchUrl;

/// One policy-judged request. The URL is a [`FetchUrl`], constructible only
/// through a [`FetchPolicy`](super::policy::FetchPolicy), so no caller of the
/// transport can ask the wire for a URL the policy did not accept.
pub struct FetchRequest {
    /// The accepted URL, as the fetcher will use it.
    pub url: FetchUrl,
    /// The resume offset for a ranged GET (the download slice): when set,
    /// the transport sends `Range: bytes=<start>-` so a partial can be
    /// continued instead of restarted. `None` is an ordinary full GET.
    pub range_start: Option<u64>,
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
    /// The response body, chunk by chunk. `None` from
    /// [`FetchBody::next_chunk`] ends it.
    pub body: Box<dyn FetchBody>,
}

/// The close- or length-delimited body of one response, pulled chunk by
/// chunk.
#[async_trait]
pub trait FetchBody: Send {
    /// The next chunk of the body, or `None` at its end. The tool applies
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
