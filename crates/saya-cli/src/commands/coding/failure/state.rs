//! The latched failure classes: first cause per class, ranked by severity.
use super::rank::class_rank;
use saya_agent::{ProgressStop, ProgressStopKind};
use saya_harness::coding::child::OwnerFailureSnapshot;
use saya_harness::coding::coordinator::{
    ChildCleanupFailure, ChildRunError, ScheduleChildError, ScheduleOutcome, classify_child_failure,
};
use saya_types::{CodingFailureClass as Class, CodingRunFailure};

#[derive(Debug, Default)]
pub(crate) struct FailureState {
    /// One entry per distinct class latched so far, keeping the first failure
    /// of each class (its attribution). The primary and secondary below are
    /// derived from it by `resort`.
    latched: Vec<CodingRunFailure>,
    /// The highest-ranked latched failure, or `None` when nothing is latched.
    pub(super) primary: Option<CodingRunFailure>,
    /// The highest-ranked latched class other than the primary's, if any.
    pub(super) secondary: Option<Class>,
    pub(super) diagnostic_fault: Option<saya_types::CodingDiagnosticFault>,
}

impl FailureState {
    /// Latches one failure: the first failure of its class is kept, and the
    /// primary and secondary are re-derived by rank. The order failures are
    /// latched in therefore never decides which class is the primary.
    pub(super) fn latch(&mut self, failure: CodingRunFailure) {
        if self.latched.iter().any(|held| held.class == failure.class) {
            return;
        }
        self.latched.push(failure);
        self.resort();
    }

    /// Derives the primary and secondary from the latched classes: the highest
    /// rank is the primary, the next highest distinct class the secondary.
    fn resort(&mut self) {
        let mut ranked: Vec<&CodingRunFailure> = self.latched.iter().collect();
        ranked.sort_by_key(|failure| std::cmp::Reverse(class_rank(failure.class)));
        let primary = ranked.first().map(|failure| (*failure).clone());
        let secondary = ranked.get(1).map(|failure| failure.class);
        self.primary = primary;
        self.secondary = secondary;
    }

    pub(super) fn latch_schedule_outcome(&mut self, outcome: &ScheduleOutcome) {
        let owner_failure = match &outcome.result {
            Err(ScheduleChildError::Run(run)) => nested_owner_failure(run),
            _ => None,
        };
        // A child's own failure class is reported in its summary, not latched as
        // the job's failure — the root can still answer. The `class` value is
        // still computed for the owner-failure comparison.
        let class = match &outcome.result {
            Err(error) => classify_child_failure(error),
            Ok(output) if output.truncated => Some(Class::OutputLimit),
            _ => None,
        };
        if let Some(owner_failure) = owner_failure {
            let attributed = |class| {
                CodingRunFailure::attributed(
                    class,
                    outcome.agent_id.clone(),
                    outcome.invocation_id.clone(),
                )
            };
            if class != Some(owner_failure.primary()) {
                self.latch(attributed(owner_failure.primary()));
            }
            if let Some(secondary) = owner_failure.secondary()
                && class != Some(secondary)
                && owner_failure.primary() != secondary
            {
                self.latch(attributed(secondary));
            }
        }
        if let Err(ScheduleChildError::Run(ChildRunError::Cleanup { kind, .. })) = &outcome.result {
            self.latch(root(match kind {
                ChildCleanupFailure::Resources => Class::EffectsUncertain,
                ChildCleanupFailure::BudgetPersistence => Class::MandatoryPersistence,
                ChildCleanupFailure::Progress => Class::ProgressDelivery,
            }));
        }
        if outcome.completion_error.is_some() {
            self.latch(root(Class::MandatoryPersistence));
        }
    }
}

pub(super) fn root(class: Class) -> CodingRunFailure {
    CodingRunFailure::root(class)
}

/// The failure a progress guard reports: its class, config key, configured
/// value, measurement, and one human line.
pub(crate) fn progress_failure(stop: ProgressStop) -> CodingRunFailure {
    let class = match stop.kind {
        ProgressStopKind::NoProgress => Class::NoProgress,
        ProgressStopKind::Repeating => Class::Repeating,
        ProgressStopKind::WallClock => Class::WallClock,
        ProgressStopKind::Cost => Class::Cost,
    };
    CodingRunFailure::root(class).with_guard(
        stop.limit,
        stop.limit_value,
        Some(stop.measured),
        stop.message,
    )
}

fn nested_owner_failure(run: &ChildRunError) -> Option<OwnerFailureSnapshot> {
    match run {
        ChildRunError::OwnerOperation { failure, .. } => Some(*failure),
        ChildRunError::Cleanup { primary, .. } => nested_owner_failure(primary),
        _ => None,
    }
}
