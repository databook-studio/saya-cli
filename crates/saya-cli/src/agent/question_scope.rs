//! The one answering budget and stop source owned by one user question.

use crate::config::runtime::RuntimeConfig;
use saya_agent::{
    AnsweringLimits, AnsweringScope, BudgetDimension, CancellationReason, CancellationToken,
};
use saya_config::BudgetLimit;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Keeps every answering invocation for a question on one admission wallet.
///
/// The elapsed source starts before connector setup. Calling
/// [`QuestionScope::finish_segment`] stops it before post-turn learning, so
/// ancillary work neither consumes nor is cancelled by answering time.
#[derive(Clone)]
pub(crate) struct QuestionScope {
    inner: Arc<QuestionInner>,
}

struct QuestionInner {
    answering: AnsweringScope,
    active_time: Mutex<ActiveTime>,
}

struct ActiveTime {
    remaining: Option<Duration>,
    active_since: Option<Instant>,
}

impl QuestionScope {
    pub(crate) fn new(runtime: &RuntimeConfig, cancellation: CancellationToken) -> Self {
        Self {
            inner: Arc::new(QuestionInner {
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
                active_time: Mutex::new(ActiveTime {
                    remaining: elapsed(runtime.resolved.investigation_budgets.elapsed_seconds),
                    active_since: None,
                }),
            }),
        }
    }

    pub(crate) fn cancellation(&self) -> CancellationToken {
        self.inner.answering.cancellation_token()
    }

    pub(crate) fn answering_scope(&self) -> AnsweringScope {
        self.inner.answering.clone()
    }

    /// Opens one exclusive charged source segment and returns its remaining
    /// deadline. No async runtime is required to construct the scope.
    pub(crate) fn begin_segment(&self) -> Option<Duration> {
        let mut active = self.inner.active_time.lock().expect("active time lock");
        assert!(
            active.active_since.is_none(),
            "overlapping question segments"
        );
        if active
            .remaining
            .is_some_and(|remaining| remaining.is_zero())
        {
            self.cancellation()
                .cancel_with_reason(CancellationReason::Budget(BudgetDimension::ElapsedTime));
            return Some(Duration::ZERO);
        }
        active.active_since = Some(Instant::now());
        active.remaining
    }

    /// Debits the current source segment once. It is safe for each early
    /// source-return path to call this because a finished segment has no start.
    pub(crate) fn finish_segment(&self) {
        let mut active = self.inner.active_time.lock().expect("active time lock");
        let Some(started) = active.active_since.take() else {
            return;
        };
        if let Some(remaining) = active.remaining.as_mut() {
            *remaining = remaining.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                self.cancellation()
                    .cancel_with_reason(CancellationReason::Budget(BudgetDimension::ElapsedTime));
            }
        }
    }
}

fn elapsed(limit: BudgetLimit) -> Option<Duration> {
    match limit {
        BudgetLimit::Finite(seconds) => Some(Duration::from_secs(seconds.get())),
        BudgetLimit::Unlimited => None,
    }
}

fn finite(limit: BudgetLimit) -> Option<u64> {
    match limit {
        BudgetLimit::Finite(value) => Some(value.get()),
        BudgetLimit::Unlimited => None,
    }
}
