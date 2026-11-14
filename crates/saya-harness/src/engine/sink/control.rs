use saya_agent::{BudgetDimension, CancellationReason, CancellationToken};

use super::EngineEventSink;

impl EngineEventSink {
    pub(super) fn request_budget_stop(&self, dimension: BudgetDimension) -> bool {
        self.cancellation.as_ref().is_some_and(|cancellation| {
            cancellation.cancel_with_reason(CancellationReason::Budget(dimension));
            true
        })
    }

    pub(in crate::engine) fn has_cancellation(&self) -> bool {
        self.cancellation.is_some()
    }

    pub(in crate::engine) fn request_owner_failure(&self) {
        if let Some(cancellation) = &self.cancellation {
            cancellation.cancel_with_reason(CancellationReason::OwnerFailure);
        }
    }

    pub(in crate::engine) fn cancellation_reason(&self) -> Option<CancellationReason> {
        self.cancellation
            .as_ref()
            .and_then(CancellationToken::reason)
    }
}
