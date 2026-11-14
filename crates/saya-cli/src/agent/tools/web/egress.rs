//! The job's one web egress: the wire every coding surface's web tools run
//! over, and the search backend they may ask.
//!
//! Built once per job and shared by the root and every helper, for the same
//! reason `web_fetch` has always been: a fetch is the same read whoever makes
//! it, and a search is the same query. What differs per surface is the
//! *consent line* — who is asking — and that is the prompt, composed at the
//! surface, not the egress.
//!
//! The search backend is optional and stays optional: [`WebEgress::search`] is
//! `None` for the ordinary case — no `[web_search]` was configured — and
//! [`super::WebTools::compose`] then composes no `web_search` tool at all.
//! Saya runs no search service of its own, so "unconfigured" is not a degraded
//! mode; it is the default.

use std::sync::Arc;

use saya_config::{ResolvedWebSearch, SecretResolver, WebSearchBackend};
use saya_harness::fetch::{
    FetchLimits, FetchPolicy, FetchTransport, HttpFetchTool, ReqwestTransport,
};

/// The production egress: the session's own policy shape over the real wire,
/// with the configured search backend when the user stated one. `None` when
/// the wire cannot be built at all, in which case no surface advertises a web
/// tool.
pub(crate) fn egress(
    resolved: &ResolvedWebSearch,
    resolver: Arc<dyn SecretResolver>,
) -> Option<Arc<WebEgress>> {
    let transport = ReqwestTransport::new().ok()?;
    Some(Arc::new(WebEgress::new(
        Arc::new(transport),
        WebEgress::search_from(resolved, resolver),
    )))
}

/// A configured search backend, with the resolver its credential reference is
/// read from. The reference is resolved at call time
/// ([`super::search::WebSearchTool::call`]), so this holds a resolver rather
/// than a value: nothing here holds a secret, and nothing here logs one.
pub(crate) struct SearchBackend {
    pub(crate) backend: WebSearchBackend,
    pub(crate) resolver: Arc<dyn SecretResolver>,
}

/// The job's one web egress. Cheap to clone — the wire is shared, not copied.
#[derive(Clone)]
pub(crate) struct WebEgress {
    transport: Arc<dyn FetchTransport>,
    search: Option<Arc<SearchBackend>>,
}

impl WebEgress {
    /// The egress over `transport`, with `search` when a backend was
    /// configured. The transport is the caller's: the job passes the
    /// production wire, a test passes its own.
    pub(crate) fn new(transport: Arc<dyn FetchTransport>, search: Option<SearchBackend>) -> Self {
        Self {
            transport,
            search: search.map(Arc::new),
        }
    }

    /// The configured search backend, or `None` where the user configured
    /// none — which is what leaves `web_search` unadvertised everywhere.
    pub(crate) fn search(&self) -> Option<Arc<SearchBackend>> {
        self.search.clone()
    }

    /// The configured search backend read from resolved configuration and the
    /// run's own secrets, or `None` where the user configured none. The one
    /// place the two are joined, so every surface sees the same backend.
    pub(crate) fn search_from(
        resolved: &ResolvedWebSearch,
        resolver: Arc<dyn SecretResolver>,
    ) -> Option<SearchBackend> {
        let backend = resolved.backend.clone()?;
        Some(SearchBackend { backend, resolver })
    }

    /// A fetch client over this egress's wire, under `limits`. The session's
    /// own policy shape (HTTPS only, the refused ranges still refused), so
    /// every client built here — the page reader's and the search lane's —
    /// judges a URL the same way.
    pub(crate) fn client(&self, limits: FetchLimits) -> Arc<HttpFetchTool> {
        Arc::new(HttpFetchTool::new(
            FetchPolicy::session(),
            Arc::clone(&self.transport),
            limits,
        ))
    }
}
