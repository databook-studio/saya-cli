//! Per-ledger schedule exclusion; aborted started work fails closed.
use super::JobBudgetLedger;
use saya_agent::{CancellationReason, CancellationToken};
use tokio::sync::OwnedSemaphorePermit;

pub(crate) struct ScheduleLease {
    _permit: OwnedSemaphorePermit,
    cancellation: CancellationToken,
    outstanding: bool,
}
impl JobBudgetLedger {
    pub(crate) fn try_schedule(&self) -> Option<ScheduleLease> {
        self.schedule
            .clone()
            .try_acquire_owned()
            .ok()
            .map(|permit| ScheduleLease {
                _permit: permit,
                cancellation: self.cancellation.clone(),
                outstanding: false,
            })
    }
}
impl ScheduleLease {
    pub(crate) fn mark_started(&mut self) {
        self.outstanding = true;
    }
    pub(crate) fn complete(mut self) {
        self.outstanding = false;
    }
}
impl Drop for ScheduleLease {
    fn drop(&mut self) {
        // Latch failure before the semaphore permit is released. Reservations
        // may remain unknown; this is not successful settlement or recovery.
        if self.outstanding {
            self.cancellation
                .cancel_with_reason(CancellationReason::OwnerFailure);
        }
    }
}
