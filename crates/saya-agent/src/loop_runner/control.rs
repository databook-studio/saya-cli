use super::{AgentLimits, admission::Admission, preflight::RequestPreflightConfig};
use crate::{
    AgentError, AgentEventSink, AgentRequest, ApprovalDecider, CancellationToken, ChatProvider,
    EffectAdmission, ToolDefinition, ToolExecutor,
};
use std::{fmt, sync::Arc};
use tokio::sync::{Mutex, OwnedMutexGuard};

/// Optional ceilings shared by every logical answering request in a run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnsweringLimits {
    pub max_requests: Option<u64>,
    pub max_tool_calls: Option<u64>,
    pub max_reported_tokens: Option<u64>,
}

/// Cloneable owner for answering limits shared by invocations of one user
/// question. Calls using one scope are serialized through its admission state.
#[derive(Clone)]
pub struct AnsweringScope {
    admission: Arc<Mutex<Admission>>,
    cancellation: CancellationToken,
}

impl fmt::Debug for AnsweringScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AnsweringScope { .. }")
    }
}

impl AnsweringScope {
    /// Creates one question scope with immutable ceilings and its shared stop
    /// signal. Every invocation in the question must use this scope.
    pub fn new(limits: AnsweringLimits, cancellation: CancellationToken) -> Self {
        Self {
            admission: Arc::new(Mutex::new(Admission::new(limits, true))),
            cancellation,
        }
    }

    /// Returns the actual stop signal used by every invocation in this scope.
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    pub(super) async fn lock(&self) -> Result<OwnedMutexGuard<Admission>, AgentError> {
        let cancellation = self.cancellation_token();
        let admission = Arc::clone(&self.admission);
        if let Ok(guard) = Arc::clone(&admission).try_lock_owned() {
            return Ok(guard);
        }
        if let Some(error) = Admission::cancellation_error(cancellation.reason(), true) {
            return Err(error);
        }
        let guard = tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                return Err(Admission::cancellation_error(cancellation.reason(), true)
                    .unwrap_or(AgentError::Cancelled));
            }
            guard = admission.lock_owned() => guard,
        };
        if let Some(error) = Admission::cancellation_error(cancellation.reason(), true) {
            return Err(error);
        }
        Ok(guard)
    }
}

/// Inputs for the controlled answering API, grouped to keep its call surface small.
pub struct AgentRunControl<'a> {
    pub(super) limits: AgentLimits,
    pub(super) answering_scope: AnsweringScope,
    pub(super) approval: &'a dyn ApprovalDecider,
    pub(super) sink: &'a dyn AgentEventSink,
    pub(super) cancellation: CancellationToken,
    pub(super) request_preflight: Option<RequestPreflightConfig>,
    pub(super) effect_admission: Option<&'a dyn EffectAdmission>,
}

impl<'a> AgentRunControl<'a> {
    pub fn new(
        limits: AgentLimits,
        answering: AnsweringLimits,
        approval: &'a dyn ApprovalDecider,
        sink: &'a dyn AgentEventSink,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            limits,
            answering_scope: AnsweringScope::new(answering, cancellation.clone()),
            approval,
            sink,
            cancellation,
            request_preflight: None,
            effect_admission: None,
        }
    }

    /// Reuses one question's admission counts and stop signal across
    /// invocations. The scope's limits and token replace this control's fresh
    /// scope and token.
    #[must_use]
    pub fn with_answering_scope(mut self, scope: AnsweringScope) -> Self {
        self.cancellation = scope.cancellation_token();
        self.answering_scope = scope;
        self
    }

    /// Applies a known model window and actual output reservation to the
    /// provider-neutral request preflight. Omitting this builder keeps a
    /// finite byte ceiling with an unknown model window.
    #[must_use]
    pub fn with_request_preflight(mut self, config: RequestPreflightConfig) -> Self {
        self.request_preflight = Some(config);
        self
    }

    /// Adds the owner that reserves logical provider requests and executable tools.
    #[must_use]
    pub fn with_effect_admission(mut self, owner: &'a dyn EffectAdmission) -> Self {
        self.effect_admission = Some(owner);
        self
    }
}

pub(super) struct AgentRunInvocation<'a> {
    pub(super) provider: &'a dyn ChatProvider,
    pub(super) tools: &'a dyn ToolExecutor,
    pub(super) request: AgentRequest,
    pub(super) definitions: Vec<ToolDefinition>,
    pub(super) limits: AgentLimits,
    pub(super) approval: &'a dyn ApprovalDecider,
    pub(super) sink: &'a dyn AgentEventSink,
    pub(super) cancellation: CancellationToken,
    pub(super) answering: Option<AnsweringScope>,
    pub(super) request_preflight: Option<RequestPreflightConfig>,
    pub(super) effect_admission: Option<&'a dyn EffectAdmission>,
}
