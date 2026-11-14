use super::JobBudgetLedger;
use saya_store::{CodingRuntimeAuthority, StoreError};
use saya_types::Sha256Digest;

impl JobBudgetLedger {
    /// Binds trusted child policy to this live owner. A recovered owner must use
    /// explicit resume_job; a stored identity is never a renewed permission.
    pub async fn bind_runtime(
        &self,
        epoch: u64,
        policy_digest: &Sha256Digest,
    ) -> Result<(), StoreError> {
        let mut state = self.inner.lock().await;
        if state.runtime_renewal_required
            || state.quiescent.load(std::sync::atomic::Ordering::Acquire)
        {
            return Err(StoreError::Conflict);
        }
        let authority = CodingRuntimeAuthority {
            epoch,
            policy_digest: policy_digest.clone(),
        };
        state
            .store
            .bind_coding_runtime(&state.job, &authority)
            .await?;
        state.runtime_authority = Some(authority);
        Ok(())
    }

    /// Called only by explicit recovery composition after the durable scan. The
    /// store repeats its blocker/tail checks atomically with epoch replacement.
    pub(crate) async fn renew_runtime(
        &self,
        fresh: CodingRuntimeAuthority,
    ) -> Result<(), StoreError> {
        let mut state = self.inner.lock().await;
        if !state.runtime_renewal_required {
            return Err(StoreError::Conflict);
        }
        let append = state
            .store
            .renew_coding_runtime(
                &state.job,
                state.runtime_authority.as_ref(),
                &fresh,
                state.sequence.checked_sub(1).ok_or(StoreError::Invalid)?,
            )
            .await?;
        let saya_store::CodingEventAppend::Appended(sequence) = append else {
            return Err(StoreError::Conflict);
        };
        if sequence.get() != state.sequence {
            return Err(StoreError::Conflict);
        }
        state.sequence = sequence
            .get()
            .checked_add(1)
            .ok_or(StoreError::LimitExceeded)?;
        state.runtime_authority = Some(fresh);
        state.runtime_renewal_required = false;
        Ok(())
    }
}
