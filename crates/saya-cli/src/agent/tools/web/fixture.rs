//! The web lane's hermetic wire: a transport that answers every request from a
//! canned response and a scripted resolution, so a fetch or a search can be
//! driven end to end with no network at all. Shared by this module's own tests,
//! by the search module's, and by the two surfaces that advertise the tools —
//! the coding root and the helper — which need a real lane to compose.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use saya_agent::{ApprovalPolicy, CancellationToken};
use saya_config::{MapSecretResolver, SecretRef, SecretResolver, WebSearchBackend};
use saya_harness::fetch::{
    FetchBody, FetchLimits, FetchPolicy, FetchRequest, FetchTransport, FetchTransportError,
    HttpFetchTool, WireResponse,
};
use saya_types::JobId;
use serde_json::{Value, json};

use super::egress::{SearchBackend, WebEgress};
use super::{WebFetchTool, WebTools, compose, prompt};
use crate::agent::tools::consent::{Caller, Operator};

/// The destination every hermetic fetch names: a host the session-wide policy
/// accepts by name, resolved below to a TEST-NET address the policy accepts.
pub(crate) const HOST: &str = "docs.example.org";

/// The address [`HOST`] resolves to — TEST-NET-1, which no refusal covers.
pub(crate) const PUBLIC_ADDRESS: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 7);

/// An address the policy refuses, as a resolver would report one.
pub(crate) const PRIVATE_ADDRESS: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 7);

/// A URL under [`HOST`].
pub(crate) fn url(path: &str) -> String {
    format!("https://{HOST}{path}")
}

/// A call naming a URL.
pub(crate) fn call(url: &str) -> Value {
    json!({ "url": url })
}

/// A call naming a URL and a text bound.
pub(crate) fn bounded_call(url: &str, max_bytes: usize) -> Value {
    json!({ "url": url, "max_bytes": max_bytes })
}

/// The body one canned response serves, one chunk and then the end.
struct CannedBody(Option<Vec<u8>>);

#[async_trait]
impl FetchBody for CannedBody {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, FetchTransportError> {
        Ok(self.0.take())
    }
}

/// A transport that answers every request with one canned response, over a
/// scripted resolution. It records what it was asked for — the URL and the
/// headers — so a test can assert both that a refused call never reached the
/// wire and that the call that did carried what it should.
pub(crate) struct CannedNet {
    status: u16,
    location: Option<String>,
    content_type: Option<String>,
    body: Vec<u8>,
    addresses: Vec<IpAddr>,
    seen: Mutex<Vec<(String, HashMap<String, String>)>>,
}

impl CannedNet {
    /// A 200 response carrying `body`, over a resolution to [`PUBLIC_ADDRESS`].
    pub(crate) fn serving(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            location: None,
            content_type: None,
            body: body.into(),
            addresses: vec![IpAddr::V4(PUBLIC_ADDRESS)],
            seen: Mutex::new(Vec::new()),
        }
    }

    /// The same, with the server's own `Content-Type`.
    pub(crate) fn typed(mut self, content_type: &str) -> Self {
        self.content_type = Some(content_type.to_owned());
        self
    }

    /// The same, over a resolution to an address the policy refuses.
    pub(crate) fn resolving_to(mut self, address: Ipv4Addr) -> Self {
        self.addresses = vec![IpAddr::V4(address)];
        self
    }

    /// The same body, served as a `302` to a host nobody stated: what an
    /// endpoint that would send the request elsewhere answers with.
    pub(crate) fn redirecting(mut self) -> Self {
        self.status = 302;
        self.location = Some("https://elsewhere.example.org/search".to_owned());
        self
    }

    /// How many GETs reached this transport.
    pub(crate) fn requests(&self) -> usize {
        self.seen.lock().unwrap().len()
    }

    /// What reached this transport, in order: each request's URL and the
    /// headers it carried. Cloned out, so a test reads a snapshot rather than
    /// holding the lock.
    pub(crate) fn seen(&self) -> Vec<(String, HashMap<String, String>)> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait]
impl FetchTransport for CannedNet {
    async fn resolve(&self, _host: &str) -> Result<Vec<IpAddr>, FetchTransportError> {
        Ok(self.addresses.clone())
    }

    async fn get(&self, request: FetchRequest) -> Result<WireResponse, FetchTransportError> {
        let headers = request
            .headers
            .iter()
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect();
        self.seen
            .lock()
            .unwrap()
            .push((request.url.as_str().to_owned(), headers));
        Ok(WireResponse {
            status: self.status,
            location: self.location.clone(),
            content_range: None,
            content_type: self.content_type.clone(),
            body: Box::new(CannedBody(Some(self.body.clone()))),
        })
    }
}

/// An operator that answers a fixed verdict and counts every call.
pub(crate) struct RecordingOperator {
    approve: bool,
    calls: Mutex<usize>,
}

impl RecordingOperator {
    pub(crate) fn new(approve: bool) -> Arc<Self> {
        Arc::new(Self {
            approve,
            calls: Mutex::new(0),
        })
    }

    pub(crate) fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

#[async_trait]
impl Operator for RecordingOperator {
    async fn ask(&self, _subject: &str, _arguments: &Value) -> bool {
        *self.calls.lock().unwrap() += 1;
        self.approve
    }
}

/// The job the fixture's prompts name.
pub(crate) fn job() -> JobId {
    JobId::parse("job-p11b1").expect("the fixture's job id is well formed")
}

/// The prompt a root's fetch is put to the operator under.
pub(crate) fn root_prompt(operator: Arc<dyn Operator>) -> crate::agent::tools::consent::Prompt {
    prompt(operator, Caller::Root { job_id: job() })
}

/// The tool over `transport` under `policy`, with an operator to answer it.
/// `None` where this mode advertises no `web_fetch` at all.
pub(crate) fn tool(
    transport: Arc<dyn FetchTransport>,
    policy: ApprovalPolicy,
    operator: Option<Arc<dyn Operator>>,
) -> Option<WebFetchTool> {
    compose(
        client(transport),
        policy,
        operator.map(root_prompt),
        CancellationToken::new(),
    )
}

/// The egress the tests drive: the session's own policy shape over a caller's
/// transport, bounded by the tool lane's limits — the same shape
/// [`super::egress`] builds for a job, with the wire supplied instead of
/// constructed.
pub(crate) fn client(transport: Arc<dyn FetchTransport>) -> Arc<HttpFetchTool> {
    Arc::new(HttpFetchTool::new(
        FetchPolicy::session(),
        transport,
        FetchLimits::for_tool_lane(),
    ))
}

/// The environment variable the fixture's Brave credential is named by.
pub(crate) const TEST_KEY_ENV: &str = "SAYA_BRAVE_API_KEY";

/// The value that variable holds. Never a real key: it exists to be asserted
/// absent from everything but the wire.
pub(crate) const TEST_KEY: &str = "test-subscription-token";

/// The SearXNG backend the fixture's tests configure: an operator's own
/// instance, on a host the session policy accepts by name.
pub(crate) fn searxng_backend() -> WebSearchBackend {
    WebSearchBackend::Searxng {
        endpoint: "https://search.example.org/search".to_owned(),
    }
}

/// The Brave backend the fixture's tests configure: Brave's own API shape,
/// with a credential **reference** — never a literal, here or anywhere.
pub(crate) fn brave_backend() -> WebSearchBackend {
    WebSearchBackend::Brave {
        endpoint: "https://api.search.brave.com/res/v1/web/search".to_owned(),
        api_key: SecretRef::Env {
            env: TEST_KEY_ENV.to_owned(),
        },
    }
}

/// The resolver a configured credential resolves through: the fixture's
/// environment holds it.
pub(crate) fn resolver() -> Arc<dyn SecretResolver> {
    Arc::new(MapSecretResolver::new([(
        TEST_KEY_ENV.to_owned(),
        TEST_KEY.to_owned(),
    )]))
}

/// The same, with the variable unset: what an unresolvable reference looks
/// like.
pub(crate) fn resolver_without_the_key() -> Arc<dyn SecretResolver> {
    Arc::new(MapSecretResolver::new([]))
}

/// The definition one of a lane's tools advertises, by name — what a test
/// hands to `approve`, exactly as the loop hands the advertised definition it
/// was given.
pub(crate) fn definition_of(lane: &WebTools, name: &str) -> saya_agent::ToolDefinition {
    lane.definitions()
        .into_iter()
        .find(|definition| definition.name == name)
        .unwrap_or_else(|| panic!("the lane advertises {name}"))
}

/// The web lane over `transport`, with `backend` configured when one is, under
/// `policy` and with an operator to answer an `ask`. `None` where this mode
/// reaches no host at all.
pub(crate) fn web_tools(
    transport: Arc<dyn FetchTransport>,
    backend: Option<WebSearchBackend>,
    policy: ApprovalPolicy,
    operator: Option<Arc<dyn Operator>>,
) -> Option<WebTools> {
    web_tools_with_resolver(transport, backend, policy, operator, resolver())
}

/// The same, with the credential resolver stated — for the tests that need a
/// reference that does not resolve.
pub(crate) fn web_tools_with_resolver(
    transport: Arc<dyn FetchTransport>,
    backend: Option<WebSearchBackend>,
    policy: ApprovalPolicy,
    operator: Option<Arc<dyn Operator>>,
    resolver: Arc<dyn SecretResolver>,
) -> Option<WebTools> {
    let search = backend.map(|backend| SearchBackend { backend, resolver });
    WebTools::compose(
        &WebEgress::new(transport, search),
        policy,
        operator,
        &Caller::Root { job_id: job() },
        CancellationToken::new(),
    )
}
