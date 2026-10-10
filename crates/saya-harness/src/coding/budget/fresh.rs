use super::ledger::BudgetLedgerError;
use saya_store::SqliteStateStore;
use saya_types::{JobId, JobState};

pub(super) async fn initial_sequence(
    store: &SqliteStateStore,
    job: &JobId,
) -> Result<u64, BudgetLedgerError> {
    if store
        .get_coding_job(job)
        .await
        .map_err(|_| BudgetLedgerError::Store)?
        .is_none_or(|record| record.state != JobState::Running)
        || !store
            .list_coding_budget_reservations(job, None, 1)
            .await
            .map_err(|_| BudgetLedgerError::Store)?
            .items
            .is_empty()
    {
        return Err(BudgetLedgerError::NotFreshRunning);
    }

    let mut cursor = None;
    let mut history = Vec::with_capacity(256);
    loop {
        let page = store
            .list_coding_events(job, cursor, 100)
            .await
            .map_err(|_| BudgetLedgerError::Store)?;
        history.extend(page.events);
        if history.len() > 256 {
            return Err(BudgetLedgerError::NotFreshRunning);
        }
        let Some(next) = page.next_cursor else { break };
        if Some(next) == cursor || history.len() == 256 {
            return Err(BudgetLedgerError::NotFreshRunning);
        }
        cursor = Some(next);
    }
    if history.iter().enumerate().any(|(index, event)| {
        event.sequence().get() != u64::try_from(index).unwrap_or(u64::MAX) + 1
    }) {
        return Err(BudgetLedgerError::NotFreshRunning);
    }
    history
        .last()
        .map_or(Some(1), |event| event.sequence().get().checked_add(1))
        .ok_or(BudgetLedgerError::Store)
}
