use super::ledger::{BudgetSnapshot, JobBudgetLedger, UsageCoverage};
use saya_agent::CancellationToken;
use saya_types::JobId;
use std::time::Duration;

impl JobBudgetLedger {
    /// Authenticated owner ceilings, including the policy restored on resume.
    pub async fn limits(&self) -> super::CountLimits {
        self.inner.lock().await.limits
    }

    pub async fn snapshot(&self) -> BudgetSnapshot {
        let state = self.inner.lock().await;
        let mut view = state.snapshot.clone();
        view.tokens_charged = state.token_policy.as_ref().map(|_| state.counts.tokens);
        view.token_limit = state
            .token_policy
            .as_ref()
            .map(|policy| policy.data().total_tokens);
        view.logical_requests = state.counts.logical;
        view.transport_attempts = state.counts.transport;
        view.tool_calls = state.counts.tools;
        view.model_slots_active = state.counts.model_slots;
        view.child_slots_active = state.counts.child_slots;
        view.command_slots_active = state.counts.command_slots;
        view.tool_permits_pending = u64::try_from(
            state
                .permits
                .values()
                .filter(|permit| permit.is_tool())
                .count(),
        )
        .unwrap_or(u64::MAX);
        view.poisoned = state.snapshot.poisoned;
        if view.usage_coverage == UsageCoverage::Complete && state.pending_physical_attempts() > 0 {
            view.usage_coverage = UsageCoverage::Partial;
        }
        view
    }

    pub async fn remaining(&self) -> Duration {
        self.inner.lock().await.remaining()
    }

    pub async fn job_id(&self) -> JobId {
        self.inner.lock().await.job.clone()
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}
