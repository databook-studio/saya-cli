//! Durable per-turn progress checks for a coding job's event log.

use super::NativeCodingJob;
use saya_agent::ProgressCheck;
use saya_types::{CodingEventKind, CodingEventV1};

impl NativeCodingJob {
    /// Appends one turn's progress check to the job's durable event log.
    /// Best effort: the check is reporting, not authority, so a store refusal
    /// leaves the run's outcome unchanged.
    pub(super) async fn record_progress_event(&self, check: ProgressCheck) {
        let event = CodingEventKind::ProgressReported {
            turn: check.turn,
            turns_without_progress: check.turns_without_progress,
            limit: check.limit,
            reset: check.reset,
            last_progress_turn: check.last_progress_turn,
            message: check.message(),
        };
        let job_id = self.job_id().clone();
        let _ = self
            .ledger
            .sequencer()
            .append_event(|sequence| {
                CodingEventV1::new(
                    std::num::NonZeroU64::new(sequence).expect("sequencer allocates nonzero"),
                    job_id.clone(),
                    None,
                    None,
                    event.clone(),
                )
                .expect("validated progress event")
            })
            .await;
    }
}
