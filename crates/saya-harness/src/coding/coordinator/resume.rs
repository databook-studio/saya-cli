use crate::coding::budget::{BudgetLedgerError, CountLimits, JobBudgetLedger};
use saya_store::{CodingRecoveryStatus, CodingRuntimeAuthority, SqliteStateStore, StoreError};
use saya_types::JobId;

#[derive(Debug, thiserror::Error)]
pub enum ResumeError {
    #[error("job requires effect, child, approval or checkout reconciliation")]
    ReconciliationRequired(CodingRecoveryStatus),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Budget(#[from] BudgetLedgerError),
}

/// Explicitly resumes admission after the caller obtains new root consent.
/// Fresh authority must have a strictly newer epoch. No child, command, write,
/// approval, or result is replayed here. Counts/unknowns come from the existing
/// ledger fold; its elapsed-time clock restarts and is not elapsed continuity.
pub async fn resume_job(
    store: SqliteStateStore,
    job: JobId,
    limits: CountLimits,
    fresh: CodingRuntimeAuthority,
) -> Result<JobBudgetLedger, ResumeError> {
    let status = super::recovery::inspect_recovery(&store, &job).await?;
    if !status.clear_for_resume() {
        return Err(ResumeError::ReconciliationRequired(status));
    }
    let ledger = JobBudgetLedger::resume(store, job, limits).await?;
    ledger.renew_runtime(fresh).await?;
    Ok(ledger)
}
