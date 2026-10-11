use super::JobBudgetLedger;
use saya_agent::EffectAdmissionError;
use saya_types::JobId;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::OwnedSemaphorePermit;

/// Exclusive local job owner for cleanup. All ledger admissions stay frozen
/// until Drop; holding this is not proof about another process or another job.
pub struct JobQuiescence {
    job: JobId,
    frozen: Arc<AtomicBool>,
    _permit: OwnedSemaphorePermit,
}
impl JobQuiescence {
    pub fn job_id(&self) -> &JobId {
        &self.job
    }
}
impl Drop for JobQuiescence {
    fn drop(&mut self) {
        self.frozen.store(false, Ordering::Release);
    }
}
impl JobBudgetLedger {
    pub async fn try_quiescent(&self) -> Result<JobQuiescence, EffectAdmissionError> {
        let permit = self
            .schedule
            .clone()
            .try_acquire_owned()
            .map_err(|_| EffectAdmissionError::Denied)?;
        let state = self.inner.lock().await;
        if state.snapshot.poisoned
            || state.cancellation.is_cancelled()
            || !state.permits.is_empty()
            || !state.local.is_empty()
            || state.counts.child_slots != 0
            || state.counts.model_slots != 0
            || state.counts.command_slots != 0
            || state.snapshot.unsettled_reservations != 0
            || state.runtime_renewal_required
        {
            return Err(EffectAdmissionError::Denied);
        }
        state.quiescent.store(true, Ordering::Release);
        Ok(JobQuiescence {
            job: state.job.clone(),
            frozen: state.quiescent.clone(),
            _permit: permit,
        })
    }
}
