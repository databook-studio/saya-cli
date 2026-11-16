//! The one adapter from the resolved `[limits.job]` table to the coding
//! engines.
//!
//! Every coding surface reads its ceilings here and nowhere else: the durable
//! ledger (`CountLimits`), the root loop (`AgentLimits`), a helper's answering
//! loop (`AnsweringLimits`), and the two facts the proposal, the child factory,
//! and `coding doctor` select on. `saya-config` resolves the table once;
//! `budget_mode = "uncapped"` resolves every dimension to `Unlimited`, so a
//! `None` below is that mode or an unstated job-wide ceiling — never a missing
//! bounded value.

use saya_agent::{AgentLimits, AnsweringLimits, ProgressLimits};
use saya_config::{BudgetLimit, ResolvedConfig, ResolvedJobLimits};
use saya_harness::coding::budget::CountLimits;
use saya_types::{CodingBudgetMode, CodingFailureClass, CodingRunFailure};
use std::time::Duration;

/// The job's budget mode. The proposal, the child factory, and the ledger all
/// select bounded versus uncapped on it.
pub(crate) fn mode(config: &ResolvedConfig) -> CodingBudgetMode {
    resolved(config).budget_mode
}

/// The resolved job ceilings, for the display report only.
pub(super) fn resolved(config: &ResolvedConfig) -> &ResolvedJobLimits {
    &config.limits.job
}

/// The optional job-wide reported-token ceiling; `None` is "no job-wide
/// ceiling", which is also what the uncapped mode resolves to.
pub(crate) fn job_token_ceiling(config: &ResolvedConfig) -> Option<u64> {
    resolved(config).job_tokens().value()
}

/// The counts one job's ledger is opened under: the `[limits.job]` ceilings
/// beside the two `[agents]` bounds, which is where the child lane is bounded.
///
/// `None` is the uncapped mode — the caller builds that ledger from the slot
/// bounds instead. A bounded resolution is finite by construction, so no
/// dimension is ever missing here.
pub(crate) fn count_limits(config: &ResolvedConfig) -> Option<CountLimits> {
    let limits = resolved(config);
    Some(CountLimits {
        logical_requests: limits.logical_requests().value()?,
        transport_attempts: limits.transport_attempts().value()?,
        tool_calls: limits.tool_calls().value()?,
        model_slots: config.agents.max_concurrent_model_requests as u64,
        child_slots: config.agents.max_children as u64,
        deadline: limits.elapsed_seconds().duration()?,
    })
}

/// The root loop's own ceilings. An uncapped mode resolves every dimension to
/// `Unlimited`, which is the `None` this engine reads as "no ceiling".
pub(crate) fn agent_limits(config: &ResolvedConfig) -> AgentLimits {
    agent_limits_for(resolved(config), config.ai.context_byte_budget)
}

/// The same numbers bound a helper's answering loop, so a helper is never
/// wider than its root.
pub(crate) fn answering_limits(config: &ResolvedConfig) -> AnsweringLimits {
    answering_limits_for(resolved(config))
}

/// The root projection, also the seam the adapter's own tests state by hand.
pub(super) fn agent_limits_for(
    limits: &ResolvedJobLimits,
    context_byte_budget: usize,
) -> AgentLimits {
    AgentLimits {
        max_turns: limits
            .logical_requests()
            .value()
            .map(|value| value as usize),
        max_tool_calls: limits.tool_calls().value().map(|value| value as usize),
        max_continuations: Some(3),
        permit_workspace_writes: true,
        permit_candidate_writes: false,
        permit_external_effects: false,
        context_byte_budget,
        progress: Some(progress_limits(limits)),
    }
}

/// The job's wall-clock ceiling, when it has one. The root shell clamps every
/// silent-read wait to what remains of it, so a quiet run cannot hold a read
/// past the job's own ceiling.
pub(crate) fn wall_clock(config: &ResolvedConfig) -> Option<Duration> {
    match resolved(config).wall_clock_seconds {
        BudgetLimit::Finite(seconds) => Some(Duration::from_secs(seconds.get())),
        BudgetLimit::Unlimited => None,
    }
}

/// The progress guards a job's root and its helpers run under. A zero count
/// disables that guard; the wall clock is exactly the configured value, and
/// an unset value is no wall-clock guard at all — attended or not.
fn progress_limits(limits: &ResolvedJobLimits) -> ProgressLimits {
    let wall_clock = match limits.wall_clock_seconds {
        BudgetLimit::Finite(seconds) => Some(Duration::from_secs(seconds.get())),
        BudgetLimit::Unlimited => None,
    };
    ProgressLimits {
        no_progress_turns: (limits.no_progress_turns > 0).then_some(limits.no_progress_turns),
        repeat_limit: (limits.repeat_limit > 0).then_some(limits.repeat_limit),
        wall_clock,
        max_cost_micros: limits.max_cost_micros,
        limit_prefix: "limits.job",
    }
}

/// The helper projection, also the seam the adapter's own tests state by hand.
pub(super) fn answering_limits_for(limits: &ResolvedJobLimits) -> AnsweringLimits {
    AnsweringLimits {
        max_requests: limits.logical_requests().value(),
        max_tool_calls: limits.tool_calls().value(),
        max_reported_tokens: limits.known_tokens().value(),
    }
}

impl super::NativeCodingJob {
    /// The backstop stop this job's own ledger recorded, if the run ended on
    /// one. Backstops are runaway bounds, so the failure names the key, its
    /// configured value, and the count the ledger admitted.
    pub(super) async fn backstop_failure(&self) -> Option<CodingRunFailure> {
        let saya_agent::CancellationReason::Budget(dimension) = self.cancellation.reason()? else {
            return None;
        };
        let limits = self.ledger.limits().await;
        let snapshot = self.ledger.snapshot().await;
        let (limit, limit_value, measured) = match dimension {
            saya_agent::BudgetDimension::LogicalRequests => (
                "limits.job.logical_requests",
                limits.logical_requests,
                Some(snapshot.logical_requests),
            ),
            saya_agent::BudgetDimension::RequestedToolCalls => (
                "limits.job.tool_calls",
                limits.tool_calls,
                Some(snapshot.tool_calls),
            ),
            saya_agent::BudgetDimension::ReportedTokens => (
                "limits.job.known_tokens",
                snapshot.token_limit,
                snapshot.tokens_charged,
            ),
            saya_agent::BudgetDimension::ElapsedTime => (
                "limits.job.elapsed_seconds",
                limits.deadline.map(|deadline| deadline.as_secs()),
                None,
            ),
            _ => return None,
        };
        Some(CodingRunFailure::root(CodingFailureClass::Backstop).with_guard(
            limit,
            limit_value,
            measured,
            format!("stopped: the {limit} backstop was reached; raise {limit} only if the job is healthy"),
        ))
    }
}
