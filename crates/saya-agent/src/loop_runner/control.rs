use super::{
    AgentLimits, ProgressHandle, admission::Admission, coding_checkpoint::AuthorityRefusalContext,
    preflight::RequestPreflightConfig,
};
use crate::{
    ActorCorrelationDigest, AgentError, AgentEventSink, AgentRequest, ApprovalDecider,
    CancellationToken, ChatProvider, CorrelationId, EffectAdmission, ToolDefinition, ToolExecutor,
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
    conversation_correlation: CorrelationId,
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
            conversation_correlation: CorrelationId::next(),
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

    pub(super) const fn conversation_correlation(&self) -> CorrelationId {
        self.conversation_correlation
    }
}

/// Inputs for the controlled answering API, grouped to keep its call surface small.
pub struct AgentRunControl<'a> {
    pub(super) mailbox: Option<super::steering::AgentMailbox>,
    pub(super) limits: AgentLimits,
    pub(super) answering_scope: AnsweringScope,
    pub(super) approval: &'a dyn ApprovalDecider,
    pub(super) sink: &'a dyn AgentEventSink,
    pub(super) cancellation: CancellationToken,
    pub(super) request_preflight: Option<RequestPreflightConfig>,
    pub(super) effect_admission: Option<&'a dyn EffectAdmission>,
    pub(super) require_physical_effect_admission: bool,
    pub(super) actor_correlation: Option<ActorCorrelationDigest>,
    pub(super) authority_refusal_context: Option<AuthorityRefusalContext>,
    pub(super) context_preamble: crate::ContextPreamble,
    pub(super) progress_report: Option<ProgressHandle>,
    /// How long one wait for a background exit may take, when this run waits
    /// instead of finishing. `None` — every attended surface — ends the turn
    /// as it always did; a headless job sets it and waits, without a model
    /// request, for the next background run to end.
    pub(super) background_wait: Option<std::time::Duration>,
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
            mailbox: None,
            limits,
            answering_scope: AnsweringScope::new(answering, cancellation.clone()),
            approval,
            sink,
            cancellation,
            request_preflight: None,
            effect_admission: None,
            require_physical_effect_admission: false,
            actor_correlation: None,
            authority_refusal_context: None,
            context_preamble: crate::ContextPreamble::Database,
            progress_report: None,
            background_wait: None,
        }
    }

    /// Frames this run's untrusted context blocks. A database session keeps the
    /// default preamble, which names the schema and the SQL safety checks; a
    /// coding job states the general untrusted-data fact instead, because its
    /// blocks are not database context.
    #[must_use]
    pub fn with_context_preamble(mut self, preamble: crate::ContextPreamble) -> Self {
        self.context_preamble = preamble;
        self
    }

    /// Receives bounded steering and pause requests at the existing turn boundary.
    #[must_use]
    pub fn with_mailbox(mut self, mailbox: super::steering::AgentMailbox) -> Self {
        self.mailbox = Some(mailbox);
        self
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

    /// Adds the owner that reserves logical provider requests and executable
    /// tools. Provider retries remain uncounted unless the strict builder is used.
    #[must_use]
    pub fn with_effect_admission(mut self, owner: &'a dyn EffectAdmission) -> Self {
        self.effect_admission = Some(owner);
        self
    }

    /// Requires the provider to admit each actual transport send before it
    /// reaches the configured endpoint. Unsupported providers fail closed.
    #[must_use]
    pub fn with_physical_effect_admission(mut self, owner: &'a dyn EffectAdmission) -> Self {
        self.effect_admission = Some(owner);
        self.require_physical_effect_admission = true;
        self
    }

    /// Adds a fixed-width, content-free actor digest to local request
    /// observations. It is never included in provider request bodies.
    #[must_use]
    pub fn with_actor_correlation(mut self, actor: ActorCorrelationDigest) -> Self {
        self.actor_correlation = Some(actor);
        self
    }

    /// Retains content-free immutable authority refusals for this exact child
    /// invocation. It cannot grant or alter authority.
    #[must_use]
    pub fn with_authority_refusal_context(mut self, context: AuthorityRefusalContext) -> Self {
        self.authority_refusal_context = Some(context);
        self
    }

    /// Publishes this run's progress report and any progress stop to `handle`
    /// after every turn. A job passes one so its result can report what the
    /// run did and why it stopped.
    #[must_use]
    pub fn with_progress_report(mut self, handle: ProgressHandle) -> Self {
        self.progress_report = Some(handle);
        self
    }

    /// Waits for a background shell exit instead of finishing when a headless
    /// model replies with no tool calls while a run is still live. `slice`
    /// bounds one wait, so cancellation and the job's wall clock are re-read
    /// between slices. An attended surface never calls this: its turn ends as
    /// it always did, and the notice rides the user's next message.
    #[must_use]
    pub fn with_background_wait(mut self, slice: std::time::Duration) -> Self {
        self.background_wait = Some(slice);
        self
    }
}

pub(super) struct AgentRunInvocation<'a> {
    pub(super) provider: &'a dyn ChatProvider,
    pub(super) tools: &'a dyn ToolExecutor,
    pub(super) request: AgentRequest,
    pub(super) definitions: Vec<ToolDefinition>,
    pub(super) mailbox: Option<super::steering::AgentMailbox>,
    pub(super) limits: AgentLimits,
    pub(super) approval: &'a dyn ApprovalDecider,
    pub(super) sink: &'a dyn AgentEventSink,
    pub(super) cancellation: CancellationToken,
    pub(super) answering: Option<AnsweringScope>,
    pub(super) request_preflight: Option<RequestPreflightConfig>,
    pub(super) effect_admission: Option<&'a dyn EffectAdmission>,
    pub(super) require_physical_effect_admission: bool,
    pub(super) actor_correlation: Option<ActorCorrelationDigest>,
    pub(super) authority_refusal_context: Option<AuthorityRefusalContext>,
    pub(super) context_preamble: crate::ContextPreamble,
    pub(super) progress_report: Option<ProgressHandle>,
    pub(super) background_wait: Option<std::time::Duration>,
}
