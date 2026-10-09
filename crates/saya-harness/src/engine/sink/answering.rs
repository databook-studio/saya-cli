use saya_agent::{AgentEventSink, AnsweringObservation};

use crate::engine::usage::{AnsweringCountError, StepAnsweringCounts};

use super::EngineEventSink;

impl EngineEventSink {
    pub(crate) fn with_answering_counts(self, counts: StepAnsweringCounts) -> Self {
        *self
            .answering_counts
            .lock()
            .expect("engine sink answering-count lock") = counts;
        self
    }

    pub(in crate::engine) fn begin_answering_step(&self, step: usize, tracked: bool) {
        self.answering_counts
            .lock()
            .expect("engine sink answering-count lock")
            .begin_step(step);
        *self
            .count_tracking
            .lock()
            .expect("engine sink answering-count tracking lock") = tracked;
    }

    pub(in crate::engine) fn remaining_answering_grant(
        &self,
        step: usize,
        turns: Option<u64>,
        tool_calls: Option<u64>,
    ) -> Result<(Option<u64>, Option<u64>), AnsweringCountError> {
        self.answering_counts
            .lock()
            .expect("engine sink answering-count lock")
            .remaining(step, turns, tool_calls)
    }

    pub(crate) async fn settle_failed_request(&self) {
        let tracked = *self.count_tracking.lock().expect("tracking lock");
        let pending = self
            .answering_counts
            .lock()
            .expect("counts lock")
            .is_pending();
        if tracked && pending {
            self.observe_answering(AnsweringObservation::ToolBatchObserved { requested: 0 })
                .await;
        }
    }
}
