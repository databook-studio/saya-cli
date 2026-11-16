use crate::{
    AgentError, AgentEventSink, AnsweringLimits, AnsweringObservation, BudgetDimension,
    CancellationReason, CancellationToken, TokenUsage,
};

pub(super) mod coding;

pub(super) struct Admission {
    pub(super) limits: AnsweringLimits,
    pub(super) requests: u64,
    pub(super) tool_calls: u64,
    pub(super) reported_tokens: Option<u64>,
    pub(super) controlled: bool,
    initialized: bool,
    pub(super) legacy: legacy::LegacyAdmission,
}

impl Admission {
    pub(super) fn new(limits: AnsweringLimits, controlled: bool) -> Self {
        Self {
            limits,
            requests: 0,
            tool_calls: 0,
            reported_tokens: None,
            controlled,
            initialized: false,
            legacy: legacy::LegacyAdmission::default(),
        }
    }

    pub(super) fn begin_invocation(&mut self) {
        self.legacy = legacy::LegacyAdmission::default();
    }

    pub(super) async fn initialize(&mut self, sink: &dyn AgentEventSink) {
        if self.controlled && !self.initialized {
            sink.observe_answering(AnsweringObservation::Initialized)
                .await;
            self.initialized = true;
        }
    }

    pub(super) fn cancellation_error(
        reason: Option<CancellationReason>,
        controlled: bool,
    ) -> Option<AgentError> {
        match reason {
            None => None,
            Some(CancellationReason::Budget(_)) if controlled => {
                Some(AgentError::Limit("answering budget stopped"))
            }
            Some(
                CancellationReason::User
                | CancellationReason::Budget(_)
                | CancellationReason::OwnerFailure,
            ) => Some(AgentError::Cancelled),
        }
    }

    pub(super) async fn request(
        &mut self,
        sink: &dyn AgentEventSink,
        token: &CancellationToken,
    ) -> Result<(), AgentError> {
        self.check(token)?;
        if self
            .limits
            .max_requests
            .is_some_and(|max| self.requests >= max)
        {
            return self.stop(token, BudgetDimension::LogicalRequests);
        }
        self.requests = self.requests.saturating_add(1);
        if self.controlled {
            sink.observe_answering(AnsweringObservation::RequestAdmitted)
                .await;
        }
        self.check(token)
    }

    pub(super) async fn observe_tools(
        &mut self,
        count: u64,
        sink: &dyn AgentEventSink,
        token: &CancellationToken,
    ) -> Result<(), AgentError> {
        if self.controlled {
            sink.observe_answering(AnsweringObservation::ToolBatchObserved { requested: count })
                .await;
        }
        self.check(token)?;
        self.tool_calls = self.tool_calls.saturating_add(count);
        Ok(())
    }

    pub(super) fn enforce_tools(&self, token: &CancellationToken) -> Result<(), AgentError> {
        self.check(token)?;
        if self
            .limits
            .max_tool_calls
            .is_some_and(|max| self.tool_calls > max)
        {
            return self.stop(token, BudgetDimension::RequestedToolCalls);
        }
        Ok(())
    }

    pub(super) fn settle(
        &mut self,
        usage: Option<TokenUsage>,
        token: &CancellationToken,
    ) -> Result<(), AgentError> {
        if let Some(usage) = usage {
            let count = usage.input_tokens.saturating_add(usage.output_tokens);
            self.reported_tokens = Some(self.reported_tokens.unwrap_or(0).saturating_add(count));
            if self
                .limits
                .max_reported_tokens
                .is_some_and(|max| self.reported_tokens.unwrap_or(0) >= max)
            {
                token.cancel_with_reason(CancellationReason::Budget(
                    BudgetDimension::ReportedTokens,
                ));
            }
        }
        self.check(token)
    }

    pub(super) fn check(&self, token: &CancellationToken) -> Result<(), AgentError> {
        match Self::cancellation_error(token.reason(), self.controlled) {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub(super) fn stopped_dimension(
        &self,
        error: &AgentError,
        token: &CancellationToken,
    ) -> Option<BudgetDimension> {
        if !matches!(error, AgentError::Limit("answering budget stopped")) {
            return None;
        }
        match token.reason() {
            Some(CancellationReason::Budget(dimension)) => Some(dimension),
            _ => None,
        }
    }

    fn stop<T>(
        &self,
        token: &CancellationToken,
        dimension: BudgetDimension,
    ) -> Result<T, AgentError> {
        token.cancel_with_reason(CancellationReason::Budget(dimension));
        match token.reason() {
            Some(CancellationReason::Budget(_)) => {
                Err(AgentError::Limit("answering budget stopped"))
            }
            Some(CancellationReason::User | CancellationReason::OwnerFailure) | None => {
                Err(AgentError::Cancelled)
            }
        }
    }

    pub(super) fn stop_preflight(
        &self,
        token: &CancellationToken,
        dimension: BudgetDimension,
    ) -> AgentError {
        match self.stop::<()>(token, dimension) {
            Err(error) => error,
            Ok(()) => unreachable!("a preflight stop always returns its typed reason"),
        }
    }
}
mod legacy;
