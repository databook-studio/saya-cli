use std::{collections::BTreeSet, error::Error, fmt, io, io::Write};

use saya_agent::{
    AgentLimits, AgentRequest, AnsweringLimits, ApprovalDecider, CancellationToken, ChatProvider,
    RequestPreflightConfig, ToolDefinition, ToolExecutor,
};
use saya_types::{MAX_NAME_CHARS, is_name_shaped};
use thiserror::Error;

use crate::coding::policy::ChildAuthorityView;

pub const MAX_CHILD_REQUEST_BYTES: usize = 64 * 1024;
pub const MAX_CHILD_REQUEST_ITEMS: usize = 32;
pub const MAX_CHILD_PROFILE_NAMES: usize = 32;
/// Matches the resolved model-name ceiling without adding a config dependency.
pub const MAX_CHILD_REQUEST_MODEL_CHARS: usize = 2_048;

/// Child-only inputs already narrowed by the trusted coordinator.
pub struct ChildExecutionSpec {
    authority: ChildAuthorityView,
    request: AgentRequest,
    agent_limits: AgentLimits,
    answering_limits: AnsweringLimits,
    request_preflight: Option<RequestPreflightConfig>,
}

impl fmt::Debug for ChildExecutionSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChildExecutionSpec")
            .field("authority", &"[narrowed]")
            .field("request", &"[redacted]")
            .field("agent_limits", &self.agent_limits)
            .field("answering_limits", &self.answering_limits)
            .field("request_preflight", &self.request_preflight)
            .finish()
    }
}

impl ChildExecutionSpec {
    /// Creates one finite invocation. The trusted coordinator must project a child-only
    /// request and resolve its profile names from trusted configuration before calling this.
    pub fn new(
        authority: ChildAuthorityView,
        request: AgentRequest,
        agent_limits: AgentLimits,
        answering_limits: AnsweringLimits,
        request_preflight: Option<RequestPreflightConfig>,
    ) -> Result<Self, ChildSpecError> {
        validate_agent_limits(agent_limits, answering_limits)?;
        validate_request(&request, agent_limits.context_byte_budget)?;
        Ok(Self {
            authority,
            request,
            agent_limits,
            answering_limits,
            request_preflight,
        })
    }

    pub fn authority(&self) -> &ChildAuthorityView {
        &self.authority
    }

    pub fn request(&self) -> &AgentRequest {
        &self.request
    }

    pub fn agent_limits(&self) -> AgentLimits {
        self.agent_limits
    }

    pub fn answering_limits(&self) -> AnsweringLimits {
        self.answering_limits
    }

    pub fn request_preflight(&self) -> Option<RequestPreflightConfig> {
        self.request_preflight
    }

    pub(crate) fn into_run_parts(
        self,
    ) -> (
        AgentRequest,
        AgentLimits,
        AnsweringLimits,
        Option<RequestPreflightConfig>,
    ) {
        (
            self.request,
            self.agent_limits,
            self.answering_limits,
            self.request_preflight,
        )
    }
}

fn validate_agent_limits(
    agent: AgentLimits,
    answering: AnsweringLimits,
) -> Result<(), ChildSpecError> {
    if agent.max_turns.is_none()
        || agent.max_tool_calls.is_none()
        || agent.max_continuations.is_none()
        || answering.max_requests.is_none()
        || answering.max_tool_calls.is_none()
        || answering.max_reported_tokens.is_none()
    {
        return Err(ChildSpecError::UnboundedLimits);
    }
    if agent.context_byte_budget > MAX_CHILD_REQUEST_BYTES {
        return Err(ChildSpecError::ContextBudgetTooLarge);
    }
    Ok(())
}

fn validate_request(
    request: &AgentRequest,
    context_byte_budget: usize,
) -> Result<(), ChildSpecError> {
    if request
        .model
        .chars()
        .take(MAX_CHILD_REQUEST_MODEL_CHARS + 1)
        .count()
        > MAX_CHILD_REQUEST_MODEL_CHARS
    {
        return Err(ChildSpecError::ModelNameTooLong);
    }
    if request
        .history
        .len()
        .saturating_add(request.context_blocks.len())
        > MAX_CHILD_REQUEST_ITEMS
    {
        return Err(ChildSpecError::TooManyRequestItems);
    }
    if request.profile_names.len() > MAX_CHILD_PROFILE_NAMES {
        return Err(ChildSpecError::TooManyProfileNames);
    }
    let mut seen = BTreeSet::new();
    for profile in &request.profile_names {
        if profile.chars().take(MAX_NAME_CHARS + 1).count() > MAX_NAME_CHARS
            || !is_name_shaped(profile)
            || !seen.insert(profile.as_str())
        {
            return Err(ChildSpecError::InvalidProfileNames);
        }
    }
    let maximum = MAX_CHILD_REQUEST_BYTES.min(context_byte_budget);
    measure_request(request, maximum)
}

fn measure_request(request: &AgentRequest, maximum: usize) -> Result<(), ChildSpecError> {
    let mut writer = BoundedWriter::new(maximum);
    match serde_json::to_writer(&mut writer, request) {
        Ok(()) => Ok(()),
        Err(_) if writer.exceeded => Err(ChildSpecError::RequestTooLarge),
        Err(_) => Err(ChildSpecError::InvalidRequest),
    }
}

struct BoundedWriter {
    written: usize,
    maximum: usize,
    exceeded: bool,
}

impl BoundedWriter {
    fn new(maximum: usize) -> Self {
        Self {
            written: 0,
            maximum,
            exceeded: false,
        }
    }
}

impl Write for BoundedWriter {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        let remaining = self.maximum.saturating_sub(self.written);
        let accepted = input.len().min(remaining);
        self.written += accepted;
        if accepted != input.len() {
            self.exceeded = true;
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "child request byte limit exceeded",
            ));
        }
        Ok(accepted)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Synchronous composition boundary for one already-narrowed child.
pub trait ChildFactory: Send + Sync {
    /// Builds the provider, scoped decider and matching toolset from this immutable view.
    /// `cancellation` is the token used by this invocation's agent loop; tools
    /// that own processes must bind their lifetime to this same token.
    fn build(
        &self,
        spec: &ChildExecutionSpec,
        cancellation: &CancellationToken,
    ) -> Result<ChildComponents, ChildFactoryError>;
}

/// Owned loop dependencies whose tool list was built from the same child view.
pub struct ChildComponents {
    pub(crate) provider: Box<dyn ChatProvider>,
    pub(crate) tools: Box<dyn ToolExecutor>,
    pub(crate) definitions: Vec<ToolDefinition>,
    pub(crate) approval: Box<dyn ApprovalDecider>,
}

impl ChildComponents {
    /// Groups the exact tools and definitions prepared from one authority view.
    pub fn new(
        provider: Box<dyn ChatProvider>,
        tools: Box<dyn ToolExecutor>,
        definitions: Vec<ToolDefinition>,
        approval: Box<dyn ApprovalDecider>,
    ) -> Self {
        Self {
            provider,
            tools,
            definitions,
            approval,
        }
    }
}

/// A factory failure that keeps its source available to the trusted caller.
pub struct ChildFactoryError(Box<dyn Error + Send + Sync>);

impl ChildFactoryError {
    pub fn new(source: impl Error + Send + Sync + 'static) -> Self {
        Self(Box::new(source))
    }
}

impl fmt::Debug for ChildFactoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ChildFactoryError { .. }")
    }
}

impl fmt::Display for ChildFactoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("child components could not be prepared")
    }
}

impl Error for ChildFactoryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.0.as_ref())
    }
}

/// Why a child execution specification failed its local bound checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ChildSpecError {
    #[error("child agent limits must be finite")]
    UnboundedLimits,
    #[error("child context byte budget exceeds the request bound")]
    ContextBudgetTooLarge,
    #[error("child request model name exceeds its character bound")]
    ModelNameTooLong,
    #[error("child request contains too many history or context items")]
    TooManyRequestItems,
    #[error("child request contains too many profile names")]
    TooManyProfileNames,
    #[error("child request profile names are invalid or duplicated")]
    InvalidProfileNames,
    #[error("child request exceeds its serialized byte bound")]
    RequestTooLarge,
    #[error("child request could not be encoded")]
    InvalidRequest,
}
