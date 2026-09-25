//! The production [`FetchTransport`]: reqwest behind the wire seam.
//!
//! Two properties are the reason this seam exists, and both live here:
//!
//! * **Redirects are never followed by the client.** The client is built
//!   with `redirect(Policy::none())` — mandatory. The *tool* judges every
//!   hop through `FetchPolicy::allow_redirect`; a client that follows its
//!   own redirects would fetch unjudged URLs and skip the policy entirely.
//! * **`resolve` returns every address.** The caller refuses the whole
//!   request if *any* returned address is refused
//!   (resolve-then-deny over all of them, before anything connects) — never
//!   the first address, never a connect-and-check-after.
//!
//! Bodies stream through [`FetchBody::next_chunk`] from
//! `Response::bytes_stream` — never buffered whole — so the caller's byte
//! bound bounds what is actually read, not what has already arrived.

use std::net::IpAddr;

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::{StreamExt, stream::BoxStream};
use reqwest::header::{CONTENT_RANGE, LOCATION, RANGE};

use super::transport::{
    FetchBody, FetchRequest, FetchTransport, FetchTransportError, WireResponse,
};

/// The production transport over one shared reqwest client.
#[derive(Clone)]
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    /// Builds the transport with the mandatory no-follow client (see the
    /// module doc for why `redirect(Policy::none())` is non-negotiable).
    pub fn new() -> Result<Self, FetchTransportError> {
        Ok(Self::with_client(build_client()?))
    }

    /// Wraps a caller-built client — the test seam: every client handed to
    /// the transport must carry the no-follow redirect policy itself.
    pub fn with_client(client: reqwest::Client) -> Self {
        Self { client }
    }

    /// The client, for tests that need to drive it directly.
    #[doc(hidden)]
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }
}

/// The one client builder. The redirect policy is set exactly here, so the
/// module test that asserts a redirect is surfaced (not followed) fails if
/// this line is ever removed.
fn build_client() -> Result<reqwest::Client, FetchTransportError> {
    reqwest::Client::builder()
        // Mandatory: the tool judges every redirect hop through
        // `FetchPolicy::allow_redirect`. A client-side policy that follows
        // redirects would fetch URLs the policy never accepted.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| FetchTransportError::Request {
            url: String::new(),
            detail: format!("building the HTTP client failed: {error}"),
        })
}

fn request_failure(url: &str, detail: String) -> FetchTransportError {
    FetchTransportError::Request {
        url: url.to_owned(),
        detail,
    }
}

#[async_trait]
impl FetchTransport for ReqwestTransport {
    /// Every address `host` resolves to, in resolution order — the caller
    /// refuses on any one of them before connecting.
    async fn resolve(&self, host: &str) -> Result<Vec<IpAddr>, FetchTransportError> {
        // An IP literal needs no DNS: resolution returns it verbatim.
        if let Ok(literal) = host.parse::<IpAddr>() {
            return Ok(vec![literal]);
        }
        let addresses = tokio::net::lookup_host((host, 0u16))
            .await
            .map_err(|_| FetchTransportError::Resolve {
                host: host.to_owned(),
            })?
            .map(|socket| socket.ip())
            .collect::<Vec<_>>();
        if addresses.is_empty() {
            return Err(FetchTransportError::Resolve {
                host: host.to_owned(),
            });
        }
        Ok(addresses)
    }

    /// One GET of a policy-judged URL. A redirect comes back as a status
    /// and `Location` for the tool to judge; it is never followed here.
    async fn get(&self, request: FetchRequest) -> Result<WireResponse, FetchTransportError> {
        let url = request.url.as_str().to_owned();
        let builder = build_request(self.client.get(url.as_str()), request.range_start);
        let response = builder
            .send()
            .await
            .map_err(|error| request_failure(&url, error.to_string()))?;
        let status = response.status().as_u16();
        let header = |name: reqwest::header::HeaderName| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        let location = header(LOCATION);
        let content_range = header(CONTENT_RANGE);
        let body = Box::new(ReqwestBody {
            // `bytes_stream` — never `bytes`: the body stays chunk-wise so
            // the caller's bound bounds what is read, not what is buffered.
            stream: response.bytes_stream().boxed(),
        });
        Ok(WireResponse {
            status,
            location,
            content_range,
            body,
        })
    }
}

/// Builds one request, ranged when a resume offset is set. Split from `get`
/// so the header wiring is testable without the wire.
fn build_request(
    builder: reqwest::RequestBuilder,
    range_start: Option<u64>,
) -> reqwest::RequestBuilder {
    match range_start {
        Some(start) => builder.header(RANGE, format!("bytes={start}-")),
        None => builder,
    }
}

/// The close- or length-delimited body of one response, pulled chunk by
/// chunk from the streaming response.
struct ReqwestBody {
    stream: BoxStream<'static, reqwest::Result<Bytes>>,
}

#[async_trait]
impl FetchBody for ReqwestBody {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, FetchTransportError> {
        match self.stream.next().await {
            Some(Ok(bytes)) => Ok(Some(bytes.to_vec())),
            Some(Err(error)) => Err(FetchTransportError::Request {
                url: String::new(),
                detail: error.to_string(),
            }),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// A redirect is surfaced as a status and `Location`, never followed:
    /// `/start` answers 302 → `/final` (which answers 200), and the response
    /// to `GET /start` must be exactly the 302. With any following policy
    /// this would arrive as the 200 of `/final`. Also proves the request is
    /// sent with the mandatory no-follow client.
    #[tokio::test]
    async fn a_redirect_is_surfaced_not_followed() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let addr = listener.local_addr().expect("listener address");
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                if socket.read(&mut byte).await.unwrap_or(0) == 0 {
                    return;
                }
                head.push(byte[0]);
            }
            let _ = socket
                .write_all(b"HTTP/1.1 302 Found\r\nLocation: /final\r\nConnection: close\r\n\r\n")
                .await;
        });
        let transport = ReqwestTransport::new().expect("client builds");
        let response = transport
            .client()
            .get(format!("http://{addr}/start"))
            .send()
            .await
            .expect("request sends");
        assert_eq!(response.status().as_u16(), 302);
        assert_eq!(response.headers().get("location").unwrap(), "/final");
    }

    /// `resolve` returns every address — `localhost` resolves to its
    /// loopback entries from the hosts data, with no network query — and
    /// the range header is wired on the built request.
    #[tokio::test]
    async fn resolve_returns_loopback_addresses_and_range_header_is_set() {
        let transport = ReqwestTransport::new().expect("client builds");
        let addresses = transport.resolve("localhost").await.expect("localhost");
        assert!(
            !addresses.is_empty(),
            "every address is returned: {addresses:?}"
        );
        for address in addresses {
            assert!(
                matches!(address, IpAddr::V4(v4) if v4.is_loopback())
                    || matches!(address, IpAddr::V6(v6) if v6.is_loopback()),
                "localhost resolves only to loopback: {address}"
            );
        }

        let client = reqwest::Client::new();
        let ranged = build_request(client.get("http://127.0.0.1/x"), Some(16))
            .build()
            .expect("request builds");
        assert_eq!(
            ranged
                .headers()
                .get(RANGE)
                .map(|value| value.to_str().expect("ascii")),
            Some("bytes=16-")
        );
        let plain = build_request(client.get("http://127.0.0.1/x"), None)
            .build()
            .expect("request builds");
        assert!(plain.headers().get(RANGE).is_none());
    }
}
