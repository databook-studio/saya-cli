//! A closed, payload-free reason a coding child failed. Never provider text: only which kind of
//! stop it was, so a failed job can be explained from its export without leaking detail.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingFailureClass {
    /// The model spent its output-token limit before answering (a cut answer).
    OutputLimit,
    /// The provider call failed (transport, timeout, malformed or refused response).
    ProviderError,
    /// The provider's response stream ended before it finished: usually a gateway or provider
    /// response time limit, or an output cap too large to finish inside it. Carries no provider text.
    ProviderStreamEnded,
    /// The agent loop reached one of its own limits.
    LimitReached,
    /// The model produced a tool call or history the loop could not use.
    InvalidToolCall,
    /// The job's shared budget or effect owner refused the next step.
    AdmissionDenied,
    /// The provider cannot supply the accounting the job requires.
    Unsupported,
    /// The child could not be set up or its channel identity did not match.
    SetupFailed,
    /// The child was cancelled by something other than the user.
    Cancelled,
    DiagnosticStorage,
    ProgressDelivery,
    MandatoryPersistence,
    /// A trusted extension's bounded protocol exchange failed.
    ExtensionExchange,
    EffectsUncertain,
    BudgetStopped,
    DependencyBlocked,
    /// The coordinator kept calling tools after no coordinator tool could advance the job.
    CoordinatorNoProgress,
    /// The coordinator's tool calls changed nothing for too long, whatever its next action said.
    CoordinatorStalled,
    /// Consecutive model turns made no progress before the job stopped.
    NoProgress,
    /// One tool call kept returning the same result, in a row, before the job
    /// stopped.
    Repeating,
    /// The job's wall-clock progress guard stopped it.
    WallClock,
    /// The job's cost progress guard stopped it.
    Cost,
    /// A runaway backstop (requests, tool calls, tokens, or elapsed time)
    /// stopped the job. Backstops are not spend targets.
    Backstop,
    /// Anything else, including owner failures.
    Internal,
}

impl CodingFailureClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OutputLimit => "output_limit",
            Self::ProviderError => "provider_error",
            Self::ProviderStreamEnded => "provider_stream_ended",
            Self::LimitReached => "limit_reached",
            Self::InvalidToolCall => "invalid_tool_call",
            Self::AdmissionDenied => "admission_denied",
            Self::Unsupported => "unsupported",
            Self::SetupFailed => "setup_failed",
            Self::Cancelled => "cancelled",
            Self::Internal => "internal",
            Self::DiagnosticStorage => "diagnostic_storage",
            Self::ProgressDelivery => "progress_delivery",
            Self::MandatoryPersistence => "mandatory_persistence",
            Self::ExtensionExchange => "extension_exchange",
            Self::EffectsUncertain => "effects_uncertain",
            Self::BudgetStopped => "budget_stopped",
            Self::DependencyBlocked => "dependency_blocked",
            Self::CoordinatorNoProgress => "coordinator_no_progress",
            Self::CoordinatorStalled => "coordinator_stalled",
            Self::NoProgress => "no_progress",
            Self::Repeating => "repeating",
            Self::WallClock => "wall_clock",
            Self::Cost => "cost",
            Self::Backstop => "backstop",
        }
    }
}

/// Safe failure identity; text from a provider, tool, or repository is never included.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingRunFailure {
    pub class: CodingFailureClass,
    pub agent_id: Option<super::AgentId>,
    pub invocation_id: Option<super::InvocationId>,
    /// The dotted config key a guard names, e.g.
    /// `limits.job.no_progress_turns`. Absent for a failure that is not a
    /// stated limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<Box<str>>,
    /// The configured value of [`Self::limit`], when the guard had one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_value: Option<u64>,
    /// What the guard measured when it fired.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measured: Option<u64>,
    /// One human line naming the key to change. Never provider or tool text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<Box<str>>,
}

impl CodingRunFailure {
    /// A failure with no agent attribution and no guard detail.
    pub fn root(class: CodingFailureClass) -> Self {
        Self {
            class,
            agent_id: None,
            invocation_id: None,
            limit: None,
            limit_value: None,
            measured: None,
            message: None,
        }
    }

    /// A failure attributed to one child invocation.
    pub fn attributed(
        class: CodingFailureClass,
        agent_id: super::AgentId,
        invocation_id: super::InvocationId,
    ) -> Self {
        Self {
            agent_id: Some(agent_id),
            invocation_id: Some(invocation_id),
            ..Self::root(class)
        }
    }

    /// Records the guard that fired: its config key, configured value, and
    /// measurement, plus the one human line.
    pub fn with_guard(
        mut self,
        limit: impl Into<String>,
        limit_value: Option<u64>,
        measured: Option<u64>,
        message: impl Into<String>,
    ) -> Self {
        self.limit = Some(limit.into().into_boxed_str());
        self.limit_value = limit_value;
        self.measured = measured;
        self.message = Some(message.into().into_boxed_str());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingDiagnosticFault {
    InvalidRecord,
    UnsafePath,
    LockContended,
    Storage,
    Recovery,
}
