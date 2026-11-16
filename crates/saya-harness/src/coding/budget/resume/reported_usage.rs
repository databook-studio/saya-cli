use saya_store::SqliteStateStore;
use saya_types::JobId;

use super::{Folded, map_resume_store};
use crate::coding::budget::ledger::{BudgetLedgerError, MAX_RESERVATIONS};

pub(super) async fn fold_reported_usage(
    store: &SqliteStateStore,
    job: &JobId,
    folded: &mut Folded,
) -> Result<(), BudgetLedgerError> {
    let mut cursor = None;
    let mut last_sequence = None;
    let mut scanned = 0_u64;
    loop {
        let page = store
            .list_coding_transport_attempts(job, cursor, 100)
            .await
            .map_err(map_resume_store)?;
        for record in &page.items {
            scanned = scanned
                .checked_add(1)
                .filter(|count| *count <= MAX_RESERVATIONS)
                .ok_or(BudgetLedgerError::ResumeHistoryTooLong)?;
            let sequence = record.reservation.reservation_sequence().get();
            if last_sequence.is_some_and(|last| sequence <= last) {
                return Err(BudgetLedgerError::ResumeRecordCorrupt);
            }
            last_sequence = Some(sequence);
            let Some(span) = &record.span else {
                continue;
            };
            let data = span.data();
            if !data.usage_complete {
                continue;
            }
            add_reported(
                &mut folded.reported_input_tokens,
                data.reported_input_tokens
                    .ok_or(BudgetLedgerError::ResumeRecordCorrupt)?,
            )?;
            add_reported(
                &mut folded.reported_output_tokens,
                data.reported_output_tokens
                    .ok_or(BudgetLedgerError::ResumeRecordCorrupt)?,
            )?;
        }
        let Some(next) = page.next_cursor else {
            break;
        };
        if cursor.is_some_and(|previous| next <= previous) {
            return Err(BudgetLedgerError::ResumeRecordCorrupt);
        }
        cursor = Some(next);
    }
    Ok(())
}

fn add_reported(total: &mut Option<u64>, amount: u64) -> Result<(), BudgetLedgerError> {
    let total = total.get_or_insert(0);
    *total = total
        .checked_add(amount)
        .ok_or(BudgetLedgerError::ResumeRecordCorrupt)?;
    Ok(())
}
