//! Native owner distinguishes reconciled historical root usage from this join.
use super::{CodingError, NativeCodingJob, follow_up::retained_root};
use saya_harness::coding::{budget::BudgetSnapshot, coordinator::ScheduleReport};

/// Whether unknown tool calls are the only unknown effect left, with nothing
/// live: the precondition for proving them historical. Unknown usage of a
/// model request or its transport attempts is not an effect and does not count
/// against it.
fn only_unknown_tool_calls_remain(snapshot: &BudgetSnapshot) -> bool {
    !(snapshot.poisoned
        || snapshot.model_slots_active != 0
        || snapshot.child_slots_active != 0
        || snapshot.command_slots_active != 0
        || snapshot.unknown_tool_calls == 0)
}

impl NativeCodingJob {
    pub(super) async fn normalize_join_accounting(
        &self,
        report: &mut ScheduleReport,
    ) -> Result<(), CodingError> {
        if !report.effects_uncertain
            || report.stop.is_some()
            || !report.not_started.is_empty()
            || report.outcomes.is_empty()
            || report.outcomes.len() > 32
            || report.outcomes.iter().any(|outcome| {
                outcome.completion_error.is_some()
                    || !outcome
                        .result
                        .as_ref()
                        .is_ok_and(|output| !output.truncated)
            })
        {
            return Ok(());
        }
        let snapshot = self.ledger.snapshot().await;
        if !only_unknown_tool_calls_remain(&snapshot) {
            return Ok(());
        }
        // The join itself has a live root permit until this method returns.
        // The immutable resume boundary separates it from retained usage facts.
        if retained_root::allows(self, &snapshot).await? {
            report.effects_uncertain = false;
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
#[path = "joined_accounting/gate_tests.rs"]
mod gate_tests;
