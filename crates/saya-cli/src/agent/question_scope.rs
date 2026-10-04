//! The one answering budget and stop source owned by one user question.

use crate::config::runtime::RuntimeConfig;
use saya_agent::{
    AnsweringLimits, AnsweringScope, BudgetDimension, CancellationReason, CancellationToken,
};
use saya_config::BudgetLimit;

/// Keeps every answering invocation for a question on one admission wallet.
///
/// The elapsed source starts before connector setup. Calling
/// [`QuestionScope::settle_answering`] stops it before post-turn learning, so
/// ancillary work neither consumes nor is cancelled by answering time.
#[derive(Clone)]
pub(crate) struct QuestionScope {
    answering: AnsweringScope,
    timer_stop: CancellationToken,
}

impl QuestionScope {
    pub(crate) fn new(runtime: &RuntimeConfig, cancellation: CancellationToken) -> Self {
        let timer_stop = CancellationToken::new();
        if let BudgetLimit::Finite(seconds) = runtime.resolved.investigation_budgets.elapsed_seconds
        {
            let stop = timer_stop.clone();
            let cancellation = cancellation.clone();
            tokio::spawn(async move {
                tokio::select! {
                    _ = stop.cancelled() => {}
                    _ = tokio::time::sleep(std::time::Duration::from_secs(seconds.get())) => {
                        cancellation.cancel_with_reason(CancellationReason::Budget(BudgetDimension::ElapsedTime));
                    }
                }
            });
        }
        Self {
            answering: AnsweringScope::new(
                AnsweringLimits {
                    max_requests: finite(
                        runtime
                            .resolved
                            .investigation_budgets
                            .logical_answering_requests,
                    ),
                    max_tool_calls: finite(
                        runtime.resolved.investigation_budgets.requested_tool_calls,
                    ),
                    max_reported_tokens: finite(
                        runtime.resolved.investigation_budgets.known_reported_tokens,
                    ),
                },
                cancellation,
            ),
            timer_stop,
        }
    }

    pub(crate) fn cancellation(&self) -> CancellationToken {
        self.answering.cancellation_token()
    }

    pub(crate) fn answering_scope(&self) -> AnsweringScope {
        self.answering.clone()
    }

    pub(crate) fn settle_answering(&self) {
        self.timer_stop.cancel();
    }
}

fn finite(limit: BudgetLimit) -> Option<u64> {
    match limit {
        BudgetLimit::Finite(value) => Some(value.get()),
        BudgetLimit::Unlimited => None,
    }
}
