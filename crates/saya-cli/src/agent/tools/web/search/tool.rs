//! The `web_search` tool: one bounded GET of the **configured** endpoint,
//! read as bounded results, delivered as an untrusted [`ContextBlock`], and
//! consented per call.
//!
//! The egress itself is the same harness fetch loop `web_fetch` drives
//! ([`HttpFetchTool::fetch_bounded_with_headers`]) — the policy's HTTPS gate,
//! the resolve-then-deny check over every returned address, the wall clock and
//! the byte bounds are the session's, unchanged. Two things differ, and both
//! are stated here:
//!
//! - **The destination is configuration, not the model.** The URL is built
//!   from the endpoint the user configured plus this call's query
//!   ([`super::query`]), so the model supplies words and never a host.
//! - **A redirect is refused, not followed.** The lane's own client is built
//!   with no redirect hops at all, so a search request that answers `302`
//!   fails typed: the query goes only to the endpoint the user stated.
//!
//! A credential — Brave's subscription token, the one backend that has one —
//! is resolved **per call** from its reference, so a broken reference is a
//! typed failure naming the reference rather than a value, and the token
//! rides the first request only (the header set is cleared before any
//! redirect, and this lane follows none).

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{
    ApprovalDecider, ApprovalPolicy, CancellationToken, ToolDefinition, ToolError,
    ToolExecutionContext, ToolExecutor,
};
use saya_config::{SecretResolver, WebSearchBackend};
use saya_harness::fetch::{
    BodyBound, FetchHeaders, FetchLimits, FetchToolError, HttpFetchTool, bounded_block_envelope,
};
use serde_json::{Map, Value, json};

use super::definition::is_web_search;
use super::{parse, query, render};
use crate::agent::tools::consent::{Caller, OwnApproval, Prompt};

/// The header Brave authenticates with. Lowercase because that is what the
/// header set accepts and what HTTP means by the name — the wire is
/// case-insensitive, so this is the same header either way.
const BRAVE_CREDENTIAL_HEADER: &str = "x-subscription-token";

/// The refusal a call earns under `ask` when no operator is present to answer
/// it. A headless job has nobody to consent to a search, and where the query
/// goes is the operator's decision; the flag that reaches the network from
/// such a driver is named, so the refusal is actionable.
pub(crate) const WEB_APPROVAL_UNAVAILABLE: &str = "searching the web needs the user's approval \
     and no operator is available here: a job or session searches the web with \
     --approval-mode bypass";

/// The refusal a call earns under `read-only`. That mode is the operator's
/// statement that nothing leaves this machine, and a query is a phrase the
/// user typed.
pub(crate) const READ_ONLY_REFUSAL: &str = "this session is read-only: nothing is searched, \
     because a search sends the user's query to a host outside this machine";

/// The refusal a call earns under `never`. A surface advertises no `web_search`
/// in that mode at all — the lane composes `None` — so a built tool never
/// reaches this; it is here so the decision stays total.
pub(crate) const NEVER_REFUSAL: &str = "this session searches nothing";

/// The line the operator's prompt opens with, in this tool's own words.
pub(crate) fn line(caller: &Caller) -> String {
    caller.line("Search the web", "search the web")
}

/// The operator and the line one caller's search is put to it under.
pub(crate) fn prompt(
    operator: Arc<dyn crate::agent::tools::consent::Operator>,
    caller: Caller,
) -> Prompt {
    Prompt::new(operator, line(&caller))
}

/// The bounds a search lane's client runs under: the tool lane's, with the
/// redirect bound closed. A search request has exactly one stated
/// destination, so a hop to anywhere else is refused rather than judged.
pub(crate) fn limits() -> FetchLimits {
    FetchLimits {
        max_redirect_hops: 0,
        ..FetchLimits::for_tool_lane()
    }
}

/// One bounded, consented search.
pub(crate) struct WebSearchTool {
    /// The lane's own fetch client, over the job's shared transport.
    client: Arc<HttpFetchTool>,
    backend: WebSearchBackend,
    /// Resolves the backend's credential reference at call time. Held here
    /// rather than resolved once, so a reference that becomes resolvable does
    /// not need the job restarted — and so a failure names the reference.
    resolver: Arc<dyn SecretResolver>,
    policy: ApprovalPolicy,
    /// The operator's approver, when one is present. `None` under `ask` is the
    /// headless shape: the call is refused rather than run unasked.
    prompt: Option<Prompt>,
    cancellation: CancellationToken,
}

impl WebSearchTool {
    pub(crate) fn new(
        client: Arc<HttpFetchTool>,
        backend: WebSearchBackend,
        resolver: Arc<dyn SecretResolver>,
        policy: ApprovalPolicy,
        prompt: Option<Prompt>,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            client,
            backend,
            resolver,
            policy,
            prompt,
            cancellation,
        }
    }

    /// The refusal this tool's mode earns for every call, or `None` where the
    /// mode serves one. Read by both `approve` and `call`.
    fn refusal(&self) -> Option<&'static str> {
        match self.policy {
            ApprovalPolicy::Bypass => None,
            ApprovalPolicy::Ask | ApprovalPolicy::AcceptEdits if self.prompt.is_some() => None,
            ApprovalPolicy::Ask | ApprovalPolicy::AcceptEdits => Some(WEB_APPROVAL_UNAVAILABLE),
            ApprovalPolicy::ReadOnly => Some(READ_ONLY_REFUSAL),
            ApprovalPolicy::Never => Some(NEVER_REFUSAL),
        }
    }

    /// The credential headers this call's request carries, resolved from the
    /// backend's reference. A backend with no credential sends none — an empty
    /// set, never a placeholder. A reference that cannot be resolved fails the
    /// call **before anything is sent**, naming the reference and never a
    /// value: a misconfigured key is a fact about the environment, not a
    /// secret to print.
    fn credentials(&self) -> Result<FetchHeaders, ToolError> {
        let mut headers = FetchHeaders::new();
        let Some(reference) = self.backend.credential() else {
            return Ok(headers);
        };
        let secret = self.resolver.resolve(reference).map_err(|_| {
            ToolError::Fetch(format!(
                "the web_search credential could not be resolved: {} is not set for this run",
                reference.redacted_label()
            ))
        })?;
        headers
            .push(BRAVE_CREDENTIAL_HEADER, secret.expose())
            .map_err(|error| {
                ToolError::Fetch(format!(
                    "the web_search credential cannot be sent as a header: {error}"
                ))
            })?;
        Ok(headers)
    }

    /// One approved call: build the request, send it to the configured
    /// endpoint, read the answer as bounded results, deliver them as the
    /// untrusted block inside an honest envelope. The name is checked here
    /// because this is also the executor's entry point.
    pub(crate) async fn call(&self, name: &str, arguments: &Value) -> Result<Value, ToolError> {
        if !is_web_search(name) {
            return Err(ToolError::UnsupportedTool);
        }
        if let Some(reason) = self.refusal() {
            return Err(ToolError::Fetch(reason.to_owned()));
        }
        let search = query::read(arguments)?;
        let headers = self.credentials()?;
        let url = query::url(&self.backend, &search);
        let outcome = self
            .client
            .fetch_bounded_with_headers(&url, BodyBound::Whole, &headers, &self.cancellation)
            .await
            .map_err(|error| ToolError::Fetch(refusal(error)))?;
        let hits = parse::hits(&self.backend, &outcome.block.body, &search)?;
        let (mut block, elided_bytes) = render::block(self.label(), &hits.hits);
        let mut fields = Map::new();
        fields.insert("query".to_owned(), Value::String(search.query.clone()));
        fields.insert("results".to_owned(), json!(hits.hits.len()));
        // What this call's own filter removed. Stated always — zero included —
        // because a model that asked for a filter must be able to tell "the
        // filter kept everything" from "the field was not reported".
        fields.insert("filtered_out".to_owned(), json!(hits.filtered_out));
        if block.truncated {
            fields.insert("truncated".to_owned(), Value::Bool(true));
            fields.insert("elided_bytes".to_owned(), json!(elided_bytes));
        }
        let cap = FetchLimits::tool_lane_cap();
        bounded_block_envelope(fields, &mut block, cap).ok_or_else(|| {
            ToolError::Fetch(format!(
                "the search results could not be delivered: even a fully truncated block exceeds \
                 the {cap}-byte tool-message cap"
            ))
        })
    }

    /// The block's label: the tool and the host it asked, which is the
    /// configured endpoint's — never a host a result named.
    fn label(&self) -> String {
        format!(
            "web-search {}",
            host_of(self.backend.endpoint()).unwrap_or_default()
        )
    }
}

/// The host a configured endpoint names. A reader: the endpoint was checked
/// when the configuration resolved, and the fetch lane's policy judges it
/// again before anything connects.
fn host_of(url: &str) -> Option<&str> {
    url.split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(url)
        .split(['/', '?', '#'])
        .next()
}

/// One egress failure as the model reads it. A redirect is the one case worth
/// its own sentence: the harness reports it as a hop count, and what the model
/// needs to know is that this lane does not follow one.
fn refusal(error: FetchToolError) -> String {
    match error {
        FetchToolError::TooManyRedirects { .. } => {
            "the search endpoint answered with a redirect: a search request goes only to the \
             endpoint the user configured"
                .to_owned()
        }
        other => other.to_string(),
    }
}

#[async_trait]
impl ApprovalDecider for WebSearchTool {
    /// `bypass` searches, the asking modes put the call to the operator — the
    /// mode pre-answers no search — and the two no-egress modes deny: the same
    /// decision [`WebSearchTool::refusal`] states, so a denial always has a
    /// reason to give.
    async fn approve(&self, _tool: &ToolDefinition, arguments: &Value) -> bool {
        match self.policy {
            ApprovalPolicy::Bypass => true,
            ApprovalPolicy::Ask | ApprovalPolicy::AcceptEdits => match &self.prompt {
                Some(prompt) => prompt.ask(arguments).await,
                None => false,
            },
            ApprovalPolicy::ReadOnly | ApprovalPolicy::Never => false,
        }
    }

    fn would_ask(&self, _tool: &ToolDefinition, _arguments: &Value) -> Option<bool> {
        Some(match self.policy {
            ApprovalPolicy::Ask | ApprovalPolicy::AcceptEdits => self.prompt.is_some(),
            ApprovalPolicy::Bypass | ApprovalPolicy::ReadOnly | ApprovalPolicy::Never => false,
        })
    }

    fn refusal_detail(&self, _tool: &ToolDefinition, _arguments: &Value) -> Option<String> {
        self.refusal().map(str::to_owned)
    }
}

#[async_trait]
impl OwnApproval for WebSearchTool {
    /// `web_search` owns exactly its own name — the one table its
    /// advertisement and its dispatch both read.
    fn owns(&self, name: &str) -> bool {
        is_web_search(name)
    }
}

#[async_trait]
impl ToolExecutor for WebSearchTool {
    async fn execute_with_context(
        &self,
        name: &str,
        arguments: Value,
        _context: ToolExecutionContext,
    ) -> Result<Value, ToolError> {
        self.call(name, &arguments).await
    }

    async fn execute(&self, name: &str, arguments: Value) -> Result<Value, ToolError> {
        self.call(name, &arguments).await
    }
}
