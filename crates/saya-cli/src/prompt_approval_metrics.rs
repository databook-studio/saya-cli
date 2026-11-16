//! The ApprovalWait phase sample the terminal approval's real wait records.

use crate::local_metrics::LocalMetrics;
use saya_agent::{AgentMetricOutcome, AgentMetricPhase, CancellationToken};
use std::io;
use std::sync::Arc;
use std::time::Instant;

/// The terminal wait itself plus the ApprovalWait sample it records: one
/// finished wait — an answer's arrival, the token's end, an answer channel
/// nobody could serve, or an input error. Automatic decisions never reach
/// the wait and never record.
pub(crate) async fn wait_and_record(
    metrics: Option<&Arc<LocalMetrics>>,
    cancellation: &CancellationToken,
) -> io::Result<Option<String>> {
    let started = Instant::now();
    let answer = crate::terminal_input::approval_line(cancellation).await;
    if let Some(metrics) = metrics {
        let outcome = match &answer {
            Ok(Some(_)) => AgentMetricOutcome::Succeeded,
            Ok(None) if cancellation.is_cancelled() => AgentMetricOutcome::Cancelled,
            Ok(None) => AgentMetricOutcome::Unavailable,
            Err(_) => AgentMetricOutcome::Failed,
        };
        metrics.record(&saya_agent::AgentMetric::Phase {
            phase: AgentMetricPhase::ApprovalWait,
            elapsed_ns: u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX),
            outcome,
        });
    }
    answer
}
