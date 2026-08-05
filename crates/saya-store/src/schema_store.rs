use crate::{
    CachedSchema, SCHEMA_VERSION, SchemaCacheEntry, SchemaStore, SqliteStateStore, StoreError,
    sqlite_support,
};
use async_trait::async_trait;
use saya_types::SchemaTree;
use std::time::{SystemTime, UNIX_EPOCH};

#[async_trait]
impl SchemaStore for SqliteStateStore {
    async fn upsert_schema(&self, profile_id: &str, schema: &SchemaTree) -> Result<(), StoreError> {
        sqlite_support::validate_profile_id(profile_id)?;
        let json = serde_json::to_string(schema).map_err(|_| StoreError::Unavailable)?;
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
        let row = sqlx::query_as::<_, (String, i64, i64)>(
            "SELECT schema_json, updated_unix_ms, version FROM schema_cache WHERE profile_id=?",
        )
        .bind(profile_id)
        .fetch_optional(self.pool().await?)
        .await
        .map_err(|_| StoreError::Unavailable)?;
        row.map(|(json, updated_unix_ms, version)| {
            Ok(CachedSchema {
                schema: serde_json::from_str(&json).map_err(|_| StoreError::Unavailable)?,
                updated_unix_ms,
                version: version as u32,
            })
        })
        .transpose()
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
