//! The `web_fetch` tool: one bounded GET, converted to text, delivered as an
//! untrusted [`ContextBlock`], and consented per call.
//!
//! The egress itself is the harness's own fetch loop
//! ([`HttpFetchTool::fetch_bounded_cancellable`]) — the policy's HTTPS gate,
//! the resolve-then-deny check over every returned address, the per-hop
//! redirect judgement, the wall clock and the redirect bound are the
//! session's, unchanged, and no second HTTP client exists here.
//!
//! What this module adds is the *lane*: which modes advertise the tool at all
//! ([`advertised`]), which calls it refuses and why ([`WebFetchTool::refusal`]),
//! who is asked ([`Prompt`], over the shared [`Caller`] line), and the two
//! bounds a fetched page is delivered under.
//!
//! # The two bounds
//!
//! A page is not one context turn, so the tool bounds it twice and says which
//! is which:
//!
//! - **The read bound** ([`max_body_bytes`], the lane's own pre-bound) keeps
//!   the *memory* a response may occupy. The harness ring keeps its head and
//!   tail and reports what it dropped. This is not a bound the call states: a
//!   page larger than it is still answered, from both of its ends.
//! - **The text bound** (`max_bytes`, stated by the call) bounds what the
//!   model actually reads, applied to the *converted* text through the same
//!   head-and-tail ring, so the elision marker the model sees sits in the
//!   text it is reading rather than inside markup the converter was about to
//!   drop.
//!
//! The read's own marker is a raw-bytes artifact and is removed before
//! conversion for exactly that reason; the call's bound is the one whose
//! marker the model reads.

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{
    ApprovalDecider, ApprovalPolicy, CancellationToken, ContextBlock, ToolDefinition, ToolError,
    ToolExecutionContext, ToolExecutor,
};
use saya_harness::fetch::{BodyBound, FetchLimits, HttpFetchTool, bounded_block_envelope};
use saya_harness::runner::{OutputRing, elision_marker};
use serde_json::{Map, Value, json};

use super::bounds::{MAX_URL_BYTES, body_cap, max_body_bytes};
use super::definition::is_web_fetch;
use super::text;
use crate::agent::tools::consent::{Caller, Operator, OwnApproval, Prompt};

/// The refusal a call earns under `ask` when no operator is present to answer
/// it — a headless job has nobody to consent to a fetch, and a fetch is the
/// one call that cannot be served any other way. The flag that reaches the
/// network from such a driver is named, so the refusal is actionable rather
/// than merely true.
pub(crate) const WEB_APPROVAL_UNAVAILABLE: &str = "fetching a web page needs the user's approval \
     and no operator is available here: a job or session fetches pages with \
     --approval-mode bypass";

/// The refusal a call earns under `read-only`. That mode is the operator's
/// statement that nothing leaves this machine, and a GET is a destination the
/// page's author chose.
pub(crate) const READ_ONLY_REFUSAL: &str = "this session is read-only: no web page is fetched, because a fetch is a request to a host \
     the page's author chose";

/// The refusal a call earns under `never`. A surface advertises no `web_fetch`
/// in that mode at all — [`compose`] returns `None` — so a built tool never
/// reaches this; it is here so the decision stays total.
pub(crate) const NEVER_REFUSAL: &str = "this session fetches no web page";

/// Whether a mode advertises `web_fetch` at all. The modes that may reach the
/// network are the ones that advertise it: `bypass`, which fetches without
/// asking, and the asking modes (`ask`, `accept-edits`) — neither pre-answers
/// a fetch — which put each URL to the operator, or, where there is no
/// operator, refuse with the reason above. `read-only` and `never` reach no
/// host, so they advertise nothing.
pub(crate) fn advertised(policy: ApprovalPolicy) -> bool {
    matches!(
        policy,
        ApprovalPolicy::Ask | ApprovalPolicy::AcceptEdits | ApprovalPolicy::Bypass
    )
}

/// The line the operator's prompt opens with, in this tool's own words.
pub(crate) fn line(caller: &Caller) -> String {
    caller.line("Fetch a web page", "fetch a web page")
}

/// The operator and the line one caller's fetch is put to it under: the shared
/// [`Prompt`] seam, worded by this tool. A caller with no operator has no
/// prompt — under `ask` that is the headless shape, and the call is refused.
pub(crate) fn prompt(operator: Arc<dyn Operator>, caller: Caller) -> Prompt {
    Prompt::new(operator, line(&caller))
}

/// Builds the tool, or `None` where this mode advertises none.
pub(crate) fn compose(
    fetch: Arc<HttpFetchTool>,
    policy: ApprovalPolicy,
    prompt: Option<Prompt>,
    cancellation: CancellationToken,
) -> Option<WebFetchTool> {
    advertised(policy).then_some(WebFetchTool {
        fetch,
        policy,
        prompt,
        cancellation,
    })
}

/// One bounded, converted, consented fetch.
pub(crate) struct WebFetchTool {
    fetch: Arc<HttpFetchTool>,
    policy: ApprovalPolicy,
    /// The operator's approver, when one is present. `None` under `ask` is the
    /// headless shape: the call is refused rather than run unasked.
    prompt: Option<Prompt>,
    /// The job's own token: a cancelled job stops waiting at once, and a
    /// fetch that completed only after cancellation returns cancelled.
    cancellation: CancellationToken,
}

impl WebFetchTool {
    /// The refusal this tool's mode earns for every call, or `None` where the
    /// mode serves one. Read by both `approve` and `call`, so the two cannot
    /// disagree about whether a call may run.
    fn refusal(&self) -> Option<&'static str> {
        match self.policy {
            ApprovalPolicy::Bypass => None,
            ApprovalPolicy::Ask | ApprovalPolicy::AcceptEdits if self.prompt.is_some() => None,
            ApprovalPolicy::Ask | ApprovalPolicy::AcceptEdits => Some(WEB_APPROVAL_UNAVAILABLE),
            ApprovalPolicy::ReadOnly => Some(READ_ONLY_REFUSAL),
            ApprovalPolicy::Never => Some(NEVER_REFUSAL),
        }
    }

    /// One approved call: fetch, convert, bound the text, deliver it as the
    /// untrusted block inside an honest envelope. The name is checked here
    /// because this is also the executor's entry point.
    pub(crate) async fn call(&self, name: &str, arguments: &Value) -> Result<Value, ToolError> {
        if !is_web_fetch(name) {
            return Err(ToolError::UnsupportedTool);
        }
        if let Some(reason) = self.refusal() {
            return Err(ToolError::Fetch(reason.to_owned()));
        }
        let url = url_argument(arguments)?;
        let cap = body_cap(arguments)?;
        let outcome = self
            .fetch
            .fetch_bounded_cancellable(
                &url,
                BodyBound::HeadTail {
                    cap: max_body_bytes(),
                },
                &self.cancellation,
            )
            .await
            .map_err(|error| ToolError::Fetch(error.to_string()))?;
        // The read's marker is placed between raw bytes and may land inside a
        // tag, where the converter would swallow it as markup. It is removed
        // here: the text is bounded on its own below, and that bound's marker
        // is the one the model reads.
        let raw = if outcome.truncated {
            outcome
                .block
                .body
                .replace(&elision_marker(outcome.dropped_bytes), "")
        } else {
            outcome.block.body
        };
        let rendered = text::render(&raw, outcome.content_type.as_deref());
        let ring = OutputRing::new(cap);
        ring.push(rendered.as_bytes());
        let (body, elided, elided_bytes, _) = ring.snapshot();
        let mut block = ContextBlock {
            label: format!("web-fetch {}", host_of(&outcome.url)),
            body,
            // Either bound having cut the page is what the block's own marker
            // reports: the model is owed the fact that the source had more,
            // whichever of the two bounds took it.
            truncated: outcome.truncated || elided,
        };
        let mut fields = Map::new();
        fields.insert("url".to_owned(), Value::String(outcome.url));
        fields.insert("status".to_owned(), json!(outcome.status));
        if let Some(content_type) = outcome.content_type {
            fields.insert("content_type".to_owned(), Value::String(content_type));
        }
        // A page served whole carries no `truncated`: the block's own marker
        // and this field say the same thing, and absence reads as "nothing was
        // cut".
        if block.truncated {
            fields.insert("truncated".to_owned(), Value::Bool(true));
        }
        // The page's own bytes that the read bound could not keep, and the
        // text bytes the call's own bound elided: two exact numbers, because
        // they measure two different things.
        fields.insert("dropped_bytes".to_owned(), json!(outcome.dropped_bytes));
        fields.insert("elided_bytes".to_owned(), json!(elided_bytes));
        let cap = FetchLimits::tool_lane_cap();
        bounded_block_envelope(fields, &mut block, cap).ok_or_else(|| {
            ToolError::Fetch(format!(
                "the fetched page could not be delivered: even a fully truncated block exceeds \
                 the {cap}-byte tool-message cap"
            ))
        })
    }
}

/// The URL one call asks for, validated before the policy ever sees it: a
/// non-string or an oversized one is refused typed rather than truncated.
fn url_argument(arguments: &Value) -> Result<String, ToolError> {
    let url = arguments
        .as_object()
        .ok_or(ToolError::ArgumentsNotObject)?
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::Fetch("invalid tool arguments: url must be a string".into()))?;
    if url.is_empty() || url.len() > MAX_URL_BYTES {
        return Err(ToolError::Fetch(format!(
            "invalid tool arguments: url must be 1 to {MAX_URL_BYTES} bytes"
        )));
    }
    Ok(url.to_owned())
}

/// The host a result's label names. A total reader: the URL has already been
/// judged by the policy, so this only ever labels what was fetched.
fn host_of(url: &str) -> &str {
    url.split_once("://")
        .map_or(url, |(_, rest)| rest)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(url)
}

#[async_trait]
impl ApprovalDecider for WebFetchTool {
    /// `bypass` fetches, the asking modes put the call to the operator — the
    /// mode pre-answers no fetch — and the two no-egress modes deny: the same
    /// decision [`WebFetchTool::refusal`] states, so a denial always has a
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
impl OwnApproval for WebFetchTool {
    /// `web_fetch` owns exactly its own name — the one table its advertisement
    /// and its dispatch both read.
    fn owns(&self, name: &str) -> bool {
        is_web_fetch(name)
    }
}

#[async_trait]
impl ToolExecutor for WebFetchTool {
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
