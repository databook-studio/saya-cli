use crate::{
    CachedSchema, SCHEMA_VERSION, SchemaCacheEntry, SchemaStore, SqliteStateStore, StoreError,
    sqlite_support,
};
use async_trait::async_trait;
use saya_types::{MAX_SCHEMA_BYTES, SchemaTree};
use std::time::{SystemTime, UNIX_EPOCH};

#[async_trait]
impl SchemaStore for SqliteStateStore {
    async fn upsert_schema(&self, profile_id: &str, schema: &SchemaTree) -> Result<(), StoreError> {
        sqlite_support::validate_profile_id(profile_id)?;
        schema.validate().map_err(|_| StoreError::LimitExceeded)?;
        // Bounded serialization: the tree streams into a writer that refuses
        // at the byte ceiling, so a valid-but-huge tree fails mid-stream —
        // never after a full `String` materialization.
        let mut capped = crate::bounded::BoundedWriter::new(Vec::new(), MAX_SCHEMA_BYTES);
        match serde_json::to_writer(&mut capped, schema) {
            Ok(()) => {}
            Err(error) if error.io_error_kind() == Some(std::io::ErrorKind::QuotaExceeded) => {
                return Err(StoreError::LimitExceeded);
            }
            Err(_) => return Err(StoreError::Unavailable),
        }
        let json = capped.into_inner();
        let json = String::from_utf8(json).map_err(|_| StoreError::Unavailable)?;
        let mut tx = self
            .pool()
            .await?
            .begin()
            .await
            .map_err(|_| StoreError::Unavailable)?;
        sqlx::query("INSERT INTO schema_cache(profile_id, schema_json, updated_unix_ms, version) VALUES (?, ?, ?, ?) ON CONFLICT(profile_id) DO UPDATE SET schema_json=excluded.schema_json, updated_unix_ms=excluded.updated_unix_ms, version=excluded.version")
            .bind(profile_id).bind(json).bind(now()).bind(i64::from(SCHEMA_VERSION)).execute(&mut *tx).await.map_err(|_| StoreError::Unavailable)?;
        tx.commit().await.map_err(|_| StoreError::Unavailable)?;
        self.secure_files()
    }
    async fn get_schema(&self, profile_id: &str) -> Result<Option<CachedSchema>, StoreError> {
        sqlite_support::validate_profile_id(profile_id)?;
        // Bounded row fetch: the length gate runs inside SQLite and the
        // capped `substr` keeps at most one byte past the ceiling on the
        // wire, so an oversized stored row refuses without materializing
        // its whole `String` — never fetch-then-measure.
        let row = sqlx::query_as::<_, (i64, Option<String>)>(
            "SELECT length(schema_json), CASE WHEN length(schema_json) <= ? THEN schema_json END FROM schema_cache WHERE profile_id=?",
        )
        .bind(i64::try_from(MAX_SCHEMA_BYTES).map_err(|_| StoreError::Unavailable)?)
        .bind(profile_id)
        .fetch_optional(self.pool().await?)
        .await
        .map_err(|_| StoreError::Unavailable)?;
        let Some((len, capped)) = row else {
            return Ok(None);
        };
        if len > i64::try_from(MAX_SCHEMA_BYTES).map_err(|_| StoreError::Unavailable)? {
            return Err(StoreError::LimitExceeded);
        }
        let Some(json) = capped else {
            return Err(StoreError::Unavailable);
        };
        let meta = sqlx::query_as::<_, (i64, i64)>(
            "SELECT updated_unix_ms, version FROM schema_cache WHERE profile_id=?",
        )
        .bind(profile_id)
        .fetch_one(self.pool().await?)
        .await
        .map_err(|_| StoreError::Unavailable)?;
        let (updated_unix_ms, version) = meta;
        let schema: SchemaTree =
            serde_json::from_str(&json).map_err(|_| StoreError::Unavailable)?;
        schema.validate().map_err(|_| StoreError::LimitExceeded)?;
        Ok(Some(CachedSchema {
            schema,
            updated_unix_ms,
            version: version as u32,
        }))
    }
    async fn invalidate_schema(&self, profile_id: &str) -> Result<(), StoreError> {
        sqlite_support::validate_profile_id(profile_id)?;
        let mut tx = self
            .pool()
            .await?
            .begin()
            .await
            .map_err(|_| StoreError::Unavailable)?;
        sqlx::query("DELETE FROM schema_cache WHERE profile_id=?")
            .bind(profile_id)
            .execute(&mut *tx)
            .await
            .map_err(|_| StoreError::Unavailable)?;
        tx.commit().await.map_err(|_| StoreError::Unavailable)?;
        self.secure_files()
    }
    async fn list_schema_metadata(&self) -> Result<Vec<SchemaCacheEntry>, StoreError> {
        let rows = sqlx::query_as::<_, (String, i64, i64)>("SELECT profile_id, updated_unix_ms, version FROM schema_cache ORDER BY updated_unix_ms DESC LIMIT 1000").fetch_all(self.pool().await?).await.map_err(|_| StoreError::Unavailable)?;
        Ok(rows
            .into_iter()
            .map(|(profile_id, updated_unix_ms, version)| SchemaCacheEntry {
                profile_id,
                updated_unix_ms,
                version: version as u32,
            })
            .collect())
    }
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as i64)
        .unwrap_or_default()
}
