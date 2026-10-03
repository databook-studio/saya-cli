use crate::{
    AgentError, AgentEventSink, AnsweringLimits, AnsweringObservation, BudgetDimension,
    CancellationReason, CancellationToken, TokenUsage,
};

pub(super) struct Admission {
    pub(super) limits: AnsweringLimits,
    pub(super) requests: u64,
    pub(super) tool_calls: u64,
    pub(super) reported_tokens: Option<u64>,
    pub(super) controlled: bool,
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
            legacy: legacy::LegacyAdmission::default(),
        }
    }

    pub(super) async fn initialize(&self, sink: &dyn AgentEventSink) {
        if self.controlled {
            sink.observe_answering(AnsweringObservation::Initialized)
                .await;
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
        match token.reason() {
            Some(CancellationReason::User) => Err(AgentError::Cancelled),
            Some(CancellationReason::Budget(_)) if !self.controlled => Err(AgentError::Cancelled),
            Some(CancellationReason::Budget(_)) => {
                Err(AgentError::Limit("answering budget stopped"))
            }
            Some(CancellationReason::OwnerFailure) => Err(AgentError::Cancelled),
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
}
mod legacy;
