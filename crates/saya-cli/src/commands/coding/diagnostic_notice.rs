//! The one notice a job states when its own diagnostics could not be written.
//!
//! Diagnostic files carry no execution authority: when a write fails the job
//! keeps its real outcome and says once, in the durable record, that its audit
//! trail is incomplete. The writer is not retried for that job, so a failing
//! disk costs one notice rather than a retry storm.

use super::NativeCodingJob;
use saya_types::{CodingDiagnosticFault, CodingEventKind, CodingEventV1};
use std::{num::NonZeroU64, sync::atomic::Ordering};

impl NativeCodingJob {
    /// Whether a degraded diagnostics fault left this run's audit trail
    /// incomplete. The job result states it; an unchanged run never does.
    pub(super) fn diagnostics_incomplete(&self) -> bool {
        self.failures().diagnostics_incomplete()
    }

    /// A failed writer is not retried for this job: every later diagnostic
    /// write is skipped, so one fault is one notice and one cost.
    pub(super) fn diagnostics_stopped(&self) -> bool {
        self.observation_failed.load(Ordering::Acquire)
    }

    /// Appends the job's single diagnostics notice: "diagnostics could not be
    /// written: <class>; the job continues without them". Best effort — the
    /// notice is reporting, not authority, so a store refusal leaves the run's
    /// outcome unchanged.
    pub(super) async fn record_diagnostics_incomplete(&self, fault: Option<CodingDiagnosticFault>) {
        let Some(fault) = fault else {
            return;
        };
        let job_id = self.job_id().clone();
        let _ = self
            .ledger
            .sequencer()
            .append_event(|sequence| {
                CodingEventV1::new(
                    NonZeroU64::new(sequence).expect("sequencer allocates nonzero"),
                    job_id.clone(),
                    None,
                    None,
                    CodingEventKind::DiagnosticsIncomplete { fault },
                )
                .expect("validated diagnostics event")
            })
            .await;
    }
}
