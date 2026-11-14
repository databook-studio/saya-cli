use saya_agent::BudgetDimension;

use crate::engine::state::RunState;

use super::EngineEventSink;

impl EngineEventSink {
    pub(super) fn observe_stop_before_display(&self) {
        if self.state() != RunState::Executing {
            return;
        }
        let already_stopped = self.cancellation_reason().is_some();
        let dimension = if already_stopped {
            None
        } else if self.tokens_exhausted() {
            Some(BudgetDimension::ReportedTokens)
        } else if self.downloads_exhausted() {
            Some(BudgetDimension::DownloadBytes)
        } else {
            None
        };
        if let Some(dimension) = dimension {
            self.request_budget_stop(dimension);
        }
        match self.observe_elapsed_clock(false) {
            Ok(true) => {
                if !already_stopped && dimension.is_none() {
                    self.request_budget_stop(BudgetDimension::ElapsedTime);
                }
            }
            Ok(false) => {
                if self
                    .deadline
                    .is_some_and(|deadline| (self.clock)() >= deadline)
                    && !already_stopped
                    && dimension.is_none()
                {
                    self.request_budget_stop(BudgetDimension::ElapsedTime);
                }
            }
            Err(error) => {
                self.hold_diagnostic(error);
                if !already_stopped && dimension.is_none() {
                    self.request_owner_failure();
                }
            }
        }
    }
}
