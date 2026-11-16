use std::num::NonZeroU64;

use saya_types::{ApprovalRequestV1, ApprovalScope};
use sqlx::{Row, Sqlite, Transaction};

use super::{approval_records::request_from_json, records::now};
use crate::StoreError;

/// Serialize only coordinator job-operation asks; child/tool approvals remain independent.
pub(super) async fn reject_pending_job_operation(
    transaction: &mut Transaction<'_, Sqlite>,
    incoming: &ApprovalRequestV1,
) -> Result<(), StoreError> {
    if incoming.agent_id().is_some()
        || !matches!(incoming.scope(), ApprovalScope::JobOperation { .. })
    {
        return Ok(());
    }
    let now = u64::try_from(now()?).map_err(|_| StoreError::Invalid)?;
    let rows = sqlx::query(
        "SELECT r.request_sequence, e.event_json FROM coding_approval_requests r JOIN coding_events e ON e.job_id=r.job_id AND e.sequence=r.request_sequence WHERE r.job_id=? AND r.closed_by_sequence IS NULL AND json_extract(e.event_json, '$.agent_id') IS NULL",
    )
    .bind(incoming.job_id().as_str())
    .fetch_all(&mut **transaction)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    for row in rows {
        let sequence: i64 = row
            .try_get("request_sequence")
            .map_err(|_| StoreError::Invalid)?;
        let sequence = u64::try_from(sequence)
            .ok()
            .and_then(NonZeroU64::new)
            .ok_or(StoreError::Invalid)?;
        let json: String = row.try_get("event_json").map_err(|_| StoreError::Invalid)?;
        let existing = request_from_json(json.as_bytes(), incoming.job_id(), sequence)?;
        if matches!(existing.scope(), ApprovalScope::JobOperation { .. })
            && existing.expires_at_unix_ms() > now
        {
            return Err(StoreError::Conflict);
        }
    }
    Ok(())
}
