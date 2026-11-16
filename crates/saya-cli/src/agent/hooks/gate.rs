//! The gate: the `ToolExecutor` decorator that puts the hook engine on a
//! surface's dispatch path.
//!
//! It wraps the surface's **own** executor, so every check that executor
//! already makes — the `[tools]` allow list, the plan posture, the
//! advertisement, the approval the loop took before calling in — has already
//! happened by the time a `PreToolUse` hook runs. That is the whole reason the
//! decorator sits outermost: a hook can refuse a call, and can never let one
//! through that the surface beneath it would not have served.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use saya_agent::{ApprovalDecider, ToolDefinition, ToolError, ToolExecutionContext, ToolExecutor};
use serde_json::Value;

use super::runner::Hooks;

/// One surface's tools, with its hooks in front of them.
pub(crate) struct HookGate {
    hooks: Arc<Hooks>,
    inner: Arc<dyn ToolExecutor>,
    /// A `SessionStart` hook's bounded stdout, offered to the model as a
    /// turn note. `None` where no hook printed anything.
    session_context: Option<String>,
}

impl HookGate {
    pub(crate) fn new(hooks: Arc<Hooks>, inner: Arc<dyn ToolExecutor>) -> Self {
        Self {
            hooks,
            inner,
            session_context: None,
        }
    }

    /// Carries the session-start context into the loop's turn note.
    pub(crate) fn with_session_context(mut self, context: Option<String>) -> Self {
        self.session_context = context;
        self
    }

    /// The pre-hook decision, then the call, then the post-hooks. The post
    /// hooks see whether the call succeeded and nothing else about it — the
    /// result is the model's, and a hook has no part in shaping it.
    ///
    /// `call` receives the name as a shared handle so every entry point can
    /// hand it to its own inner call without borrowing this frame.
    async fn gated<F, Fut>(&self, name: &str, arguments: Value, call: F) -> Result<Value, ToolError>
    where
        F: FnOnce(Arc<str>, Value) -> Fut,
        Fut: std::future::Future<Output = Result<Value, ToolError>>,
    {
        self.hooks.pre_tool_use(name, &arguments).await?;
        let outcome = call(Arc::from(name), arguments.clone()).await;
        self.hooks
            .post_tool_use(name, &arguments, outcome.is_ok())
            .await;
        outcome
    }
}

#[async_trait]
impl ToolExecutor for HookGate {
    async fn settle_invocation(&self) -> Result<(), ToolError> {
        self.inner.settle_invocation().await
    }

    /// The loop this gate served has ended: run the `Stop` hooks through the
    /// same runner the root uses. They observe; they change nothing.
    async fn turn_finished(&self) {
        self.hooks.stop().await;
    }

    fn turn_context_note(&self) -> Option<String> {
        match (
            self.inner.turn_context_note(),
            self.session_context.as_deref(),
        ) {
            (Some(note), Some(context)) => Some(format!("{context}\n\n{note}")),
            (None, Some(context)) => Some(context.to_owned()),
            (Some(note), None) => Some(note),
            (None, None) => None,
        }
    }

    fn advertised_definitions(&self) -> Option<Vec<ToolDefinition>> {
        self.inner.advertised_definitions()
    }

    fn take_background_exits(&self) -> Vec<saya_agent::BackgroundExit> {
        self.inner.take_background_exits()
    }

    fn background_live(&self) -> bool {
        self.inner.background_live()
    }

    fn background_notice_pending(&self) -> bool {
        self.inner.background_notice_pending()
    }

    async fn wait_background_exit(
        &self,
        deadline: std::time::Instant,
        cancellation: &saya_agent::CancellationToken,
    ) -> bool {
        self.inner
            .wait_background_exit(deadline, cancellation)
            .await
    }

    async fn execute(&self, name: &str, arguments: Value) -> Result<Value, ToolError> {
        let inner = Arc::clone(&self.inner);
        self.gated(name, arguments, move |name, arguments| async move {
            inner.execute(&name, arguments).await
        })
        .await
    }

    async fn execute_with_result_cap(
        &self,
        name: &str,
        arguments: Value,
        result_cap: usize,
    ) -> Result<Value, ToolError> {
        let inner = Arc::clone(&self.inner);
        self.gated(name, arguments, move |name, arguments| async move {
            inner
                .execute_with_result_cap(&name, arguments, result_cap)
                .await
        })
        .await
    }

    async fn execute_with_context(
        &self,
        name: &str,
        arguments: Value,
        context: ToolExecutionContext,
    ) -> Result<Value, ToolError> {
        let inner = Arc::clone(&self.inner);
        self.gated(name, arguments, move |name, arguments| async move {
            inner.execute_with_context(&name, arguments, context).await
        })
        .await
    }
}

/// The approval decorator: a `PermissionRequest` hook runs only where the
/// operator was about to be asked, and can only refuse that ask. Exit `2`
/// denies with the hook's own bounded, redacted stderr; exit `0` falls through
/// to the same operator ask the surface would have made. A decider that cannot
/// say whether it would ask (`would_ask` is `None`) is never pre-empted.
pub(crate) struct HookApprovalGate {
    hooks: Arc<Hooks>,
    inner: Arc<dyn ApprovalDecider>,
    /// The blocking hook's reason for the call the loop is about to report as
    /// denied. Set by `approve` and read once by `refusal_detail`.
    denial: Mutex<Option<String>>,
}

impl HookApprovalGate {
    pub(crate) fn new(hooks: Arc<Hooks>, inner: Arc<dyn ApprovalDecider>) -> Self {
        Self {
            hooks,
            inner,
            denial: Mutex::new(None),
        }
    }

    fn record(&self, reason: Option<String>) {
        if let Ok(mut denial) = self.denial.lock() {
            *denial = reason;
        }
    }
}

#[async_trait]
impl ApprovalDecider for HookApprovalGate {
    async fn approve(&self, tool: &ToolDefinition, arguments: &Value) -> bool {
        self.record(None);
        if self.inner.would_ask(tool, arguments) == Some(true)
            && let Err(reason) = self.hooks.permission_request(&tool.name, arguments).await
        {
            self.record(Some(reason));
            return false;
        }
        self.inner.approve(tool, arguments).await
    }

    fn would_ask(&self, tool: &ToolDefinition, arguments: &Value) -> Option<bool> {
        self.inner.would_ask(tool, arguments)
    }

    fn refusal_detail(&self, tool: &ToolDefinition, arguments: &Value) -> Option<String> {
        self.denial
            .lock()
            .ok()
            .and_then(|denial| denial.clone())
            .or_else(|| self.inner.refusal_detail(tool, arguments))
    }

    fn immutable_authority_refusal(
        &self,
        tool: &ToolDefinition,
        arguments: &Value,
    ) -> Option<saya_agent::ImmutableAuthorityRefusal> {
        self.inner.immutable_authority_refusal(tool, arguments)
    }
}
