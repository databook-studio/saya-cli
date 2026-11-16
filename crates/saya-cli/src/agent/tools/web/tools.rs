//! The web lane one surface holds: `web_fetch`, `web_search`, or both.
//!
//! Every surface that advertises a web tool holds this one value, for the same
//! reason it held `web_fetch` alone before: the advertisement, the dispatch,
//! and the approval must read the same table, and a surface that held two
//! options could disagree with itself about which of them it offers.
//!
//! # What is advertised, and why
//!
//! - **The mode.** `read-only` and `never` reach no host, so [`compose`]
//!   returns `None` and the surface holds no web lane at all.
//! - **The configuration.** Saya runs no search service of its own, so
//!   `web_search` is offered only where the user configured a backend. An
//!   unconfigured surface still fetches pages; it simply has nothing to
//!   search with, and advertises nothing that would fail.
//!
//! Both rules are answered here, once, from the egress and the mode — so no
//! surface can advertise a tool it could not serve.
//!
//! [`compose`]: WebTools::compose

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{
    ApprovalDecider, ApprovalPolicy, CancellationToken, ToolDefinition, ToolError,
    ToolExecutionContext, ToolExecutor,
};
use saya_harness::fetch::FetchLimits;
use serde_json::Value;

use super::egress::WebEgress;
use super::search;
use super::{WebFetchTool, is_web_fetch};
use crate::agent::tools::consent::{Caller, Operator, OwnApproval};

/// The web tools one surface advertises. Both halves are optional and the
/// reasons are different: the fetch half follows the mode, the search half the
/// configuration. A surface with neither holds no `WebTools` at all.
pub(crate) struct WebTools {
    fetch: Option<WebFetchTool>,
    search: Option<search::WebSearchTool>,
}

impl WebTools {
    /// Composes the lane over `egress` under `policy`, or `None` where this
    /// mode reaches no host.
    ///
    /// The operator and the caller are taken rather than a prompt, because the
    /// two tools word their own line: the operator is asked to *fetch a page*
    /// or to *search the web*, and each tool owns that wording. One operator,
    /// one caller, two lines — and no surface has to know either.
    pub(crate) fn compose(
        egress: &WebEgress,
        policy: ApprovalPolicy,
        operator: Option<Arc<dyn Operator>>,
        caller: &Caller,
        cancellation: CancellationToken,
    ) -> Option<Self> {
        if !super::advertised(policy) {
            return None;
        }
        let fetch = super::compose(
            egress.client(FetchLimits::for_tool_lane()),
            policy,
            operator
                .clone()
                .map(|ask| super::prompt(ask, caller.clone())),
            cancellation.clone(),
        );
        let search = egress.search().map(|configured| {
            search::WebSearchTool::new(
                // The search lane's own client: the same wire and the same
                // policy, with the redirect bound closed — a search has one
                // stated destination and follows no hop to another.
                egress.client(search::limits()),
                configured.backend.clone(),
                Arc::clone(&configured.resolver),
                policy,
                operator.map(|ask| search::prompt(ask, caller.clone())),
                cancellation,
            )
        });
        (fetch.is_some() || search.is_some()).then_some(Self { fetch, search })
    }

    /// The definitions this lane advertises, in one list: the fetch tool's
    /// when it was composed, the search tool's when a backend was configured.
    /// Whether the user configured a search backend. The web lane's presence
    /// says the mode reaches a host; this says the operator enabled web in
    /// configuration, which is what advertises the web tools without a URL in
    /// the request.
    pub(crate) fn search_configured(&self) -> bool {
        self.search.is_some()
    }

    pub(crate) fn definitions(&self) -> Vec<ToolDefinition> {
        let mut definitions = Vec::new();
        if self.fetch.is_some() {
            definitions.push(super::fetch_definition());
        }
        if self.search.is_some() {
            definitions.push(search::definition());
        }
        definitions
    }

    /// Serves one call, or refuses it as unknown. The name tables are the same
    /// ones the definitions came from, so a name this lane advertises is a
    /// name it serves, and no other name reaches a tool.
    pub(crate) async fn call(&self, name: &str, arguments: &Value) -> Result<Value, ToolError> {
        if is_web_fetch(name) {
            let Some(fetch) = &self.fetch else {
                return Err(ToolError::UnsupportedTool);
            };
            return fetch.call(name, arguments).await;
        }
        if search::is_web_search(name) {
            let Some(search) = &self.search else {
                return Err(ToolError::UnsupportedTool);
            };
            return search.call(name, arguments).await;
        }
        Err(ToolError::UnsupportedTool)
    }

    /// The decider for a call this lane owns, so a surface with one decider
    /// entry point does not have to know which half answers.
    fn decider(&self, name: &str) -> Option<&dyn ApprovalDecider> {
        if is_web_fetch(name) {
            return self
                .fetch
                .as_ref()
                .map(|fetch| fetch as &dyn ApprovalDecider);
        }
        if search::is_web_search(name) {
            return self
                .search
                .as_ref()
                .map(|search| search as &dyn ApprovalDecider);
        }
        None
    }
}

#[async_trait]
impl ToolExecutor for WebTools {
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

#[async_trait]
impl ApprovalDecider for WebTools {
    /// The half that owns the call decides it — each tool's own mode rule —
    /// and a name this lane advertises no tool for is denied.
    async fn approve(&self, tool: &ToolDefinition, arguments: &Value) -> bool {
        match self.decider(&tool.name) {
            Some(decider) => decider.approve(tool, arguments).await,
            None => false,
        }
    }

    fn would_ask(&self, tool: &ToolDefinition, arguments: &Value) -> Option<bool> {
        self.decider(&tool.name)
            .and_then(|decider| decider.would_ask(tool, arguments))
    }

    fn refusal_detail(&self, tool: &ToolDefinition, arguments: &Value) -> Option<String> {
        self.decider(&tool.name)
            .and_then(|decider| decider.refusal_detail(tool, arguments))
    }
}

#[async_trait]
impl OwnApproval for WebTools {
    /// The lane owns both web names and nothing else, so a surface that wraps
    /// its own decider routes exactly these calls here.
    fn owns(&self, name: &str) -> bool {
        super::is_web_tool(name)
    }
}
