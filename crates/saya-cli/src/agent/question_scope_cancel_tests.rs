//! The application-side stop instant: the first stop wins, a budget-style
//! stop without a request instant records nothing, and the settlement sample
//! is recorded exactly once.

use crate::agent::question_scope::QuestionScope;
use crate::config::runtime::RuntimeConfig;
use crate::local_metrics::{LocalMetrics, PhaseCell, phase_index};
use saya_agent::{AgentMetricPhase, BudgetDimension, CancellationReason, CancellationToken};
use saya_config::{ConnectionsFile, ResolutionInput, resolve};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc, thread, time::Duration};

fn scope_with(
    cancellation: CancellationToken,
    collector: Option<Arc<LocalMetrics>>,
) -> (QuestionScope, Option<Arc<LocalMetrics>>) {
    let connections = ConnectionsFile::default();
    let runtime = RuntimeConfig {
        resolved: resolve(ResolutionInput::new(connections.clone())).expect("test config resolves"),
        connections,
        config_path: None,
        connections_path: None,
        cache_scope: PathBuf::from("/tmp/saya-question-scope-cancel-tests"),
        investigations_root: PathBuf::from("/tmp/saya-question-scope-cancel-tests/investigations"),
        secret_values: BTreeMap::new(),
    };
    let kept = collector.as_ref().map(Arc::clone);
    let scope = QuestionScope::with_metrics(&runtime, cancellation, collector);
    (scope, kept)
}
fn cancellation_cell(collector: &Option<Arc<LocalMetrics>>) -> Option<PhaseCell> {
    let collector = collector.as_ref()?;
    let index =
        phase_index(AgentMetricPhase::CancellationToLocalSettlement).expect("known phase indexed");
    Some(collector.snapshot().phases[index])
}

fn cancellation_samples(collector: &Option<Arc<LocalMetrics>>) -> u64 {
    cancellation_cell(collector).map_or(0, |cell| cell.samples)
}

#[test]
fn question_scope_cancel_first_stop_instant_wins_over_a_later_stop() {
    let (scope, collector) = scope_with(
        CancellationToken::new(),
        Some(Arc::new(LocalMetrics::new())),
    );
    scope.request_stop();
    thread::sleep(Duration::from_millis(20));
    scope.request_stop();
    scope.record_stop_settlement();
    assert_eq!(cancellation_samples(&collector), 1);
    let cell = cancellation_cell(&collector).expect("a sample was recorded");
    let gap_ns =
        u64::try_from(Duration::from_millis(20).as_nanos()).expect("20ms fits in u64 nanoseconds");
    assert!(
        cell.max_ns >= gap_ns,
        "the first stop instant must be kept: recorded {} ns is below the 20ms gap",
        cell.max_ns
    );
    assert_eq!(
        scope.cancellation().reason(),
        Some(CancellationReason::User)
    );
}
#[test]
fn question_scope_cancel_settlement_without_a_request_records_nothing() {
    let (scope, collector) = scope_with(
        CancellationToken::new(),
        Some(Arc::new(LocalMetrics::new())),
    );
    scope
        .cancellation()
        .cancel_with_reason(CancellationReason::Budget(BudgetDimension::ElapsedTime));
    scope.record_stop_settlement();
    assert_eq!(
        cancellation_samples(&collector),
        0,
        "a core-originated stop has no request instant and records nothing"
    );
}

#[test]
fn question_scope_cancel_records_one_sample_across_a_double_settlement() {
    let (scope, collector) = scope_with(
        CancellationToken::new(),
        Some(Arc::new(LocalMetrics::new())),
    );
    scope.request_stop();
    scope.record_stop_settlement();
    scope.record_stop_settlement();
    assert_eq!(
        cancellation_samples(&collector),
        1,
        "the second settlement call must not add a sample"
    );
}
#[test]
fn question_scope_cancel_without_a_collector_still_cancels_once() {
    let (scope, collector) = scope_with(CancellationToken::new(), None);
    scope.request_stop();
    scope.record_stop_settlement();
    assert_eq!(cancellation_samples(&collector), 0);
    assert_eq!(
        scope.cancellation().reason(),
        Some(CancellationReason::User)
    );
}
