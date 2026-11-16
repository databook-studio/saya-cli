//! Phase-sample recording for the operation owner: the shared record
//! helpers, the lease outcome mapping, and the schema attempt its caller
//! turns into exactly one SchemaDiscovery sample after the cached-fallback
//! decision.

use std::time::{Duration, Instant};

use saya_agent::{AgentMetric, AgentMetricOutcome, AgentMetricPhase as Phase};
use saya_types::SchemaTree;

use crate::local_metrics::LocalMetrics;

use super::operation::OperationError;

/// Records one phase sample measured from `started` when a collector rides
/// on the entry.
pub(super) fn record(
    metrics: Option<&LocalMetrics>,
    phase: Phase,
    started: Instant,
    outcome: AgentMetricOutcome,
) {
    record_elapsed(metrics, phase, started.elapsed(), outcome);
}

/// Records an interval already measured when its window closed: the caller
/// settles the outcome later, so re-reading elapsed time from a start
/// instant would also cover that decision work.
pub(super) fn record_elapsed(
    metrics: Option<&LocalMetrics>,
    phase: Phase,
    elapsed: Duration,
    outcome: AgentMetricOutcome,
) {
    if let Some(metrics) = metrics {
        metrics.record(&AgentMetric::Phase {
            phase,
            elapsed_ns: u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX),
            outcome,
        });
    }
}

/// An acquired lease wait succeeded; a cancelled acquire did not.
pub(super) fn wait_outcome(acquired: bool) -> AgentMetricOutcome {
    if acquired {
        AgentMetricOutcome::Succeeded
    } else {
        AgentMetricOutcome::Cancelled
    }
}

/// Records the live schema attempt's own measured window under the one
/// outcome the caller settled on (`Succeeded`, `Failed`, `Cancelled`, or
/// `CachedFallback`).
pub(super) fn record_schema_discovery(
    metrics: Option<&LocalMetrics>,
    elapsed: Duration,
    outcome: AgentMetricOutcome,
) {
    record_elapsed(metrics, Phase::SchemaDiscovery, elapsed, outcome);
}

/// One live schema attempt handed back to its caller. Either the lease was
/// never granted — the acquire path only fails on cancellation, so no
/// discovery window opened and no SchemaDiscovery sample may be recorded —
/// or the window is measured and the caller records exactly one sample once
/// its cached-fallback decision is known.
pub(super) enum SchemaAttempt {
    NotStarted,
    Measured(Duration, Result<SchemaTree, OperationError>),
}
