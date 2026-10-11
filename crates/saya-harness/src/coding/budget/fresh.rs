use super::ledger::{BudgetLedgerError, CountLimits};
use saya_store::SqliteStateStore;
use saya_types::{BudgetLimitPolicyV1, JobId, JobState};

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

/// The proposed limits as the durable policy identity. The deadline is
/// converted to whole milliseconds with a checked conversion: a
/// sub-millisecond-only deadline (zero whole milliseconds) and a deadline
/// above `u64` milliseconds are refused as invalid limits instead of being
/// lossily cast or truncated.
pub(super) fn proposed_policy(
    limits: &CountLimits,
) -> Result<BudgetLimitPolicyV1, BudgetLedgerError> {
    let deadline_millis = u64::try_from(limits.deadline.as_millis())
        .ok()
        .filter(|millis| *millis > 0)
        .ok_or(BudgetLedgerError::InvalidLimits)?;
    BudgetLimitPolicyV1::new(
        limits.logical_requests,
        limits.transport_attempts,
        limits.tool_calls,
        limits.model_slots,
        limits.child_slots,
        deadline_millis,
    )
    .map_err(|_| BudgetLedgerError::InvalidLimits)
}

/// The resume path's next event sequence, derived exactly as
/// [`initial_sequence`] derives it: a bounded contiguous history of at
/// most 256 events, allocated as last + 1. Over the bound the resume
/// fails closed with [`BudgetLedgerError::ResumeHistoryTooLong`]; a
/// non-contiguous history is
/// [`BudgetLedgerError::ResumeRecordCorrupt`].
pub(super) async fn resume_sequence(
    store: &SqliteStateStore,
    job: &JobId,
) -> Result<u64, BudgetLedgerError> {
    const MAX_RESUME_EVENTS: u64 = 65_536;
    let mut cursor = None;
    let mut expected = 1_u64;
    loop {
        let page = store
            .list_coding_events(job, cursor, 100)
            .await
            .map_err(|_| BudgetLedgerError::Store)?;
        for event in &page.events {
            if expected > MAX_RESUME_EVENTS {
                return Err(BudgetLedgerError::ResumeHistoryTooLong);
            }
            if event.sequence().get() != expected {
                return Err(BudgetLedgerError::ResumeRecordCorrupt);
            }
            expected += 1;
        }
        let Some(next) = page.next_cursor else {
            return Ok(expected);
        };
        if Some(next) == cursor {
            return Err(BudgetLedgerError::ResumeRecordCorrupt);
        }
        cursor = Some(next);
    }
}
