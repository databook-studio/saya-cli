//! The run's fetch toolset adapter: `http_fetch` and `http_download` behind
//! the [`ToolExecutor`] seam the per-step composites dispatch through.
//!
//! The adapter is built per step from that step's [`FetchPolicy`] (the
//! step's approved destinations — the thing the approval view showed) over
//! the run's shared transport, workspace, and download wallet, so definition
//! and enforcement are built from the same capabilities in one place. The
//! tools it wraps stay untouched: `HttpFetchTool` produces the untrusted
//! [`ContextBlock`] (this adapter renders it, it does not re-derive it), and
//! `http_download` streams to the workspace under the run's budget — its
//! result is metadata, never page bytes.
//!
//! The lane discipline (S2 decision 1): `http_fetch`'s result text is the
//! sentinel-wrapped, escaped, labelled block rendered by
//! `saya_agent::render_untrusted_block` — the same renderer the initial
//! user turn uses, so the escape scheme is never forked — inside an honest
//! JSON envelope. The wired body bound ([`FetchLimits::for_tool_lane`])
//! sits below the loop's tool-message cap, so a fetch success always fits
//! the lane; [`bounded_fetch_envelope`] is the invariant that remains:
//! **an unclosed block is never emitted**, so the loop's own truncation
//! marker — which appends outside any block structure — can never orphan
//! the closing sentinel and let a hostile page's tail read as the tool's
//! own prose.

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{ContextBlock, ToolError, ToolExecutor, render_untrusted_block};
use serde_json::Value;

use super::budget::DownloadBudget;
use super::download::http_download;
use super::download_error::DownloadError;
use super::download_limits::DownloadLimits;
use super::limits::FetchLimits;
use super::policy::FetchPolicy;
use super::tools::{FetchToolError, HttpFetchTool};
use super::transport::FetchTransport;
use crate::workspace::Workspace;

/// The two fetch tools as one step's executor member: the policy is the
/// step's, the transport, workspace, and download wallet are the run's
/// shared ones (the wallet by clone — a trip anywhere is seen everywhere).
/// The workspace is the download destination; `None` is the session shape
/// where `http_fetch` needs no root and the download tool is simply not
/// advertised — a stray call refuses typed rather than guessing at a
/// destination.
pub struct FetchTools {
    fetch: HttpFetchTool,
    policy: FetchPolicy,
    transport: Arc<dyn FetchTransport>,
    download_limits: DownloadLimits,
    workspace: Option<Arc<Workspace>>,
    budget: DownloadBudget,
}

impl FetchTools {
    /// Builds the step's fetch member from the step's approved policy, the
    /// run's shared transport and workspace, the lane-bounded fetch limits,
    /// and the run's download wallet.
    pub fn new(
        policy: FetchPolicy,
        transport: Arc<dyn FetchTransport>,
        limits: FetchLimits,
        download_limits: DownloadLimits,
        workspace: Option<Arc<Workspace>>,
        budget: DownloadBudget,
    ) -> Self {
        Self {
            fetch: HttpFetchTool::new(policy.clone(), Arc::clone(&transport), limits),
            policy,
            transport,
            download_limits,
            workspace,
            budget,
        }
    }

    /// One bounded fetch, delivered as the honest envelope around the
    /// rendered untrusted block.
    async fn run_fetch(&self, arguments: Value) -> Result<Value, ToolError> {
        let url = string_argument(&arguments, "url")?;
        let outcome = self.fetch.fetch(&url).await.map_err(lane_error)?;
        let mut block = outcome.block;
        let cap = FetchLimits::tool_lane_cap();
        bounded_fetch_envelope(&outcome.url, &mut block, cap).ok_or_else(|| {
            ToolError::Fetch(format!(
                "the fetched page could not be delivered: even a fully truncated block \
                 exceeds the {cap}-byte tool-message cap"
            ))
        })
    }

    /// One bounded, resumable download into the workspace under the shared
    /// budget. The result is metadata — destination, bytes, digest — never
    /// page bytes. Without a workspace bound there is no destination: the
    /// call refuses typed, naming the missing root.
    async fn run_download(&self, arguments: Value) -> Result<Value, ToolError> {
        let Some(workspace) = self.workspace.as_ref() else {
            return Err(ToolError::Fetch(
                "no workspace is bound, so there is nowhere to download to: bind a \
                 workspace (a git worktree or --workspace) to download files"
                    .into(),
            ));
        };
        let url = string_argument(&arguments, "url")?;
        let destination = string_argument(&arguments, "destination")?;
        let outcome = http_download(
            &self.policy,
            self.transport.as_ref(),
            workspace,
            &destination,
            &url,
            self.download_limits,
            &self.budget,
        )
        .await
        .map_err(ToolError::from)?;
        Ok(serde_json::json!({
            "destination": outcome.destination,
            "bytes": outcome.bytes,
            "sha256": outcome.sha256,
        }))
    }
}

#[async_trait]
impl ToolExecutor for FetchTools {
    async fn execute(&self, name: &str, arguments: Value) -> Result<Value, ToolError> {
        match name {
            "http_fetch" => self.run_fetch(arguments).await,
            "http_download" => self.run_download(arguments).await,
            _ => Err(ToolError::UnsupportedTool),
        }
    }
}

/// One required string argument, validated before the policy ever sees it.
fn string_argument(arguments: &Value, name: &str) -> Result<String, ToolError> {
    arguments
        .as_object()
        .ok_or(ToolError::ArgumentsNotObject)?
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ToolError::Fetch(format!("invalid tool arguments: {name} must be a string")))
}

/// Builds the fetch result's envelope — `{"url": <final url>, "content":
/// <the rendered untrusted block>}` — under the loop's tool-message cap.
/// The normal path is the pre-bound: the wired `max_total_bytes` sits below
/// the cap minus envelope slack, so the first render fits and `truncated`
/// stays false by construction (a success is always the whole body). The
/// defensive backstop covers the pathological remainder — a hostile label,
/// escaping growth, a tighter cap: the block is flagged `truncated` so the
/// block's own marker renders inside the delimiters, and the *body* is cut
/// at a char boundary and re-rendered until the envelope fits. The floor —
/// the envelope with an empty body — is measured first, and the allowance
/// converges geometrically on any measured overshoot, so escaping and JSON
/// growth of any factor still terminate at a closed block. `None` means
/// even the empty-body block cannot fit (an absurd final URL): the caller
/// fails the call typed rather than emitting an unclosed block.
fn bounded_fetch_envelope(url: &str, block: &mut ContextBlock, cap: usize) -> Option<Value> {
    let build = |block: &ContextBlock| serde_json::json!({ "url": url, "content": render_untrusted_block(block) });
    let full = build(block);
    if full.to_string().len() <= cap {
        return Some(full);
    }
    // Every backstop render carries the in-block truncation marker, so the
    // floor — everything the envelope needs besides body bytes — includes it.
    block.truncated = true;
    let empty = build(&ContextBlock {
        label: block.label.clone(),
        body: String::new(),
        truncated: true,
    });
    let floor = empty.to_string().len();
    if floor > cap {
        return None;
    }
    // The rendered body can cost several raw bytes per character (the
    // escape scheme doubles backslashes, the JSON string doubles them
    // again), so start at a quarter of the room and converge geometrically:
    // the allowance reaches the floor before it reaches zero, and the loop
    // terminates with a fitting, closed block.
    let mut allowance = (cap - floor) / 4;
    loop {
        block
            .body
            .truncate(floor_char_boundary(&block.body, allowance));
        let candidate = build(block);
        if candidate.to_string().len() <= cap {
            return Some(candidate);
        }
        if allowance == 0 {
            return None;
        }
        allowance /= 2;
    }
}

/// Every typed fetch failure surfaces to the model as the lane's detail —
/// the refusal, the bound, the status — the same rule the runner tool
/// follows; the loop wraps it in the tool message.
fn lane_error(error: FetchToolError) -> ToolError {
    ToolError::Fetch(error.to_string())
}

/// Largest byte index `<= idx` on a UTF-8 character boundary, so a
/// backstop cut never splits a multi-byte sequence.
/// (`str::floor_char_boundary` is stable only since 1.91, above the MSRV.)
fn floor_char_boundary(text: &str, mut idx: usize) -> usize {
    if idx >= text.len() {
        idx = text.len();
    }
    while idx > 0 && !text.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

/// The download errors ride the same lane mapping as the fetch errors.
impl From<DownloadError> for ToolError {
    fn from(error: DownloadError) -> Self {
        ToolError::Fetch(error.to_string())
    }
}

#[cfg(test)]
#[path = "adapter_tests.rs"]
mod tests;
