pub(crate) use crate::local_metrics_index::{
    count_index, coverage_index, outcome_index, phase_index,
};
use saya_agent::{AgentMetric, AgentMetricUseful};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

static ENABLED: AtomicBool = AtomicBool::new(false);

pub(crate) fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub(crate) fn set_enabled(value: bool) {
    ENABLED.store(value, Ordering::Relaxed);
}

const PHASE_SLOTS: usize = 14;
const OUTCOME_SLOTS: usize = 6;
const COUNT_SLOTS: usize = 5;
const COVERAGE_SLOTS: usize = 3;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PhaseCell {
    pub(crate) samples: u64,
    pub(crate) total_ns: u64,
    pub(crate) max_ns: u64,
    pub(crate) outcomes: [u64; OUTCOME_SLOTS],
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CoverageCell {
    pub(crate) seen: bool,
    pub(crate) reported: u64,
    pub(crate) missing: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LocalPhaseSnapshot {
    pub(crate) phases: [PhaseCell; PHASE_SLOTS],
    pub(crate) counts: [u64; COUNT_SLOTS],
    pub(crate) coverage: [CoverageCell; COVERAGE_SLOTS],
    pub(crate) first_useful: Option<(AgentMetricUseful, u64)>,
    pub(crate) unrecognized: u64,
}

pub(crate) struct LocalMetrics {
    origin: Instant,
    state: Mutex<LocalPhaseSnapshot>,
}

impl LocalMetrics {
    pub(crate) fn new() -> Self {
        Self {
            origin: Instant::now(),
            state: Mutex::new(LocalPhaseSnapshot::default()),
        }
    }

    pub(crate) fn snapshot(&self) -> LocalPhaseSnapshot {
        *self.state.lock().expect("local metrics lock")
    }

    pub(crate) fn record(&self, metric: &AgentMetric) {
        let now_ns = u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(u64::MAX);
        let mut state = self.state.lock().expect("local metrics lock");
        match *metric {
            AgentMetric::Phase {
                phase,
                elapsed_ns,
                outcome,
            } => {
                let Some(index) = phase_index(phase) else {
                    state.unrecognized = state.unrecognized.saturating_add(1);
                    return;
                };
                let cell = &mut state.phases[index];
                cell.samples = cell.samples.saturating_add(1);
                cell.total_ns = cell.total_ns.saturating_add(elapsed_ns);
                cell.max_ns = cell.max_ns.max(elapsed_ns);
                if let Some(slot) = outcome_index(outcome) {
                    cell.outcomes[slot] = cell.outcomes[slot].saturating_add(1);
                } else {
                    state.unrecognized = state.unrecognized.saturating_add(1);
                }
            }
            AgentMetric::Count(count) => match count_index(count) {
                Some(index) => state.counts[index] = state.counts[index].saturating_add(1),
                None => state.unrecognized = state.unrecognized.saturating_add(1),
            },
            AgentMetric::Useful(kind) => {
                if state.first_useful.is_none() {
                    state.first_useful = Some((kind, now_ns));
                }
            }
            AgentMetric::UsageCoverage {
                attempt_outcome,
                reported,
            } => {
                let Some(index) = coverage_index(attempt_outcome) else {
                    state.unrecognized = state.unrecognized.saturating_add(1);
                    return;
                };
                let cell = &mut state.coverage[index];
                cell.seen = true;
                if reported {
                    cell.reported = cell.reported.saturating_add(1);
                } else {
                    cell.missing = cell.missing.saturating_add(1);
                }
            }
            _ => state.unrecognized = state.unrecognized.saturating_add(1),
        }
    }
}
