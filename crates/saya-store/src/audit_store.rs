use crate::{
    AuditEntry, AuditOperation, AuditRecord, AuditStatus, AuditStore, SqliteStateStore, StoreError,
    sqlite_support,
};
use async_trait::async_trait;
use std::time::{SystemTime, UNIX_EPOCH};

const AUDIT_RETENTION: i64 = 1_000;
const MAX_AUDIT_READ: usize = 1_000;

#[async_trait]
impl AuditStore for SqliteStateStore {
    async fn record_audit(&self, entry: AuditEntry) -> Result<(), StoreError> {
        sqlite_support::validate_profile_id(&entry.profile_id)?;
        if let Some(id) = &entry.session_id {
            sqlite_support::validate_session_id(id)?;
        }
        let mut tx = self
            .pool()
            .await?
            .begin()
            .await
            .map_err(|_| StoreError::Unavailable)?;
        sqlx::query("INSERT INTO audit_log(created_unix_ms, session_id, profile_id, operation, status, duration_ms, row_count, truncated) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(now()).bind(entry.session_id).bind(entry.profile_id).bind(entry.operation.as_str()).bind(entry.status.as_str()).bind(entry.duration_ms as i64).bind(entry.row_count.map(|value| value as i64)).bind(entry.truncated.map(i64::from)).execute(&mut *tx).await.map_err(|_| StoreError::Unavailable)?;
        sqlx::query("DELETE FROM audit_log WHERE id NOT IN (SELECT id FROM audit_log ORDER BY id DESC LIMIT ?)").bind(AUDIT_RETENTION).execute(&mut *tx).await.map_err(|_| StoreError::Unavailable)?;
        tx.commit().await.map_err(|_| StoreError::Unavailable)?;
        self.secure_files()
    }
    async fn recent_audit(&self, limit: usize) -> Result<Vec<AuditRecord>, StoreError> {
        let rows = sqlx::query_as::<_, (i64, Option<String>, String, String, String, i64, Option<i64>, Option<i64>)>("SELECT created_unix_ms, session_id, profile_id, operation, status, duration_ms, row_count, truncated FROM audit_log ORDER BY id DESC LIMIT ?")
            .bind(limit.clamp(1, MAX_AUDIT_READ) as i64).fetch_all(self.pool().await?).await.map_err(|_| StoreError::Unavailable)?;
        rows.into_iter().map(decode).collect()
    }
}
type AuditRow = (
    i64,
    Option<String>,
    String,
    String,
    String,
    i64,
    Option<i64>,
    Option<i64>,
);
fn decode(row: AuditRow) -> Result<AuditRecord, StoreError> {
    let (
        created_unix_ms,
        session_id,
        profile_id,
        operation,
        status,
        duration_ms,
        row_count,
        truncated,
    ) = row;
    Ok(AuditRecord {
        created_unix_ms,
        event: AuditEntry {
            profile_id,
            operation: AuditOperation::parse(&operation).ok_or(StoreError::Unavailable)?,
            status: AuditStatus::parse(&status).ok_or(StoreError::Unavailable)?,
            duration_ms: duration_ms as u64,
            row_count: row_count.map(|value| value as usize),
            truncated: truncated.map(|value| value != 0),
            session_id,
        },
    })
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as i64)
        .unwrap_or_default()
}
