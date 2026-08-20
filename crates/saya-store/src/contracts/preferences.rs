//! Phase 5c-1 — the preferences store.
//!
//! A preference is *not a claim*: it has no object, no schema fingerprint, and
//! no drift, so it lives in its own table at `user_version = 3`, not in
//! `ClaimPayload`. One value per kind per scope; setting again replaces.
//! Preferences carry no lifecycle, evidence, or audit trail — a user changing
//! their timezone is not a fact that needs provenance.
//!
//! The value is still persisted user input, so it goes through the same
//! admission gate as a claim payload: a row whose `value_json` looks like a
//! credential or a SQL statement is refused on read-back, even though the typed
//! value could never have produced it.

use crate::SqliteStateStore;
use crate::StoreError;
use crate::contracts::admission;
use crate::contracts::now;
use async_trait::async_trait;
use saya_types::{PreferenceScope, PreferenceValue};

/// The maximum serialized size of a preference value. Preferences are tiny — a
/// timezone, a grain, a style, a profile name — so a small cap catches a
/// hand-injected blob without constraining any real value.
pub const MAX_PREFERENCE_VALUE_BYTES: usize = 1024;

#[async_trait]
pub trait PreferenceStore: Send + Sync {
    /// Stores `value` at `scope`, replacing any prior value for the same kind.
    /// A value offered at the wrong scope is a typed error, not a silent coercion.
    async fn set_preference(
        &self,
        scope: &PreferenceScope,
        value: PreferenceValue,
    ) -> Result<(), StoreError>;
    /// Reads the value of `kind` at `scope`, or `None` if unset. A row whose
    /// persisted JSON fails admission is `Invalid`, not returned as data.
    async fn get_preference(
        &self,
        scope: &PreferenceScope,
        kind: &str,
    ) -> Result<Option<PreferenceValue>, StoreError>;
    /// Removes `kind` at `scope`. A missing kind is a no-op, not an error.
    async fn unset_preference(&self, scope: &PreferenceScope, kind: &str)
    -> Result<(), StoreError>;
    /// Every `(kind, value)` pair at `scope`. Rows failing admission are
    /// skipped, not surfaced — a listing must not error out on one bad row.
    async fn list_preferences(
        &self,
        scope: &PreferenceScope,
    ) -> Result<Vec<(String, PreferenceValue)>, StoreError>;
}

#[async_trait]
impl PreferenceStore for SqliteStateStore {
    async fn set_preference(
        &self,
        scope: &PreferenceScope,
        value: PreferenceValue,
    ) -> Result<(), StoreError> {
        // The store is the authority for the scope rule: a value offered at the
        // wrong scope is refused here, even though a caller using `Scoped` has
        // already validated. Defence in depth — a row written by direct SQL or
        // an older build must still fail closed on read.
        if !value.matches_scope(scope) {
            return Err(StoreError::Invalid);
        }
        let key = scope_key(scope)?;
        let serialized = serde_json::to_string(&value).map_err(|_| StoreError::Invalid)?;
        if serialized.len() > MAX_PREFERENCE_VALUE_BYTES {
            return Err(StoreError::LimitExceeded);
        }
        // A preference is bounded and typed, but it is still persisted user
        // input: reuse the payload admission gate. Refuse, do not redact-and-store.
        if crate::redact(&serialized) != serialized {
            return Err(StoreError::Invalid);
        }
        admission::check(&serialized)?;
        let mut tx = self
            .pool()
            .await?
            .begin()
            .await
            .map_err(|_| StoreError::Unavailable)?;
        sqlx::query("INSERT INTO user_preferences(scope_key, preference_kind, value_json, updated_unix_ms) VALUES (?, ?, ?, ?) ON CONFLICT(scope_key, preference_kind) DO UPDATE SET value_json=excluded.value_json, updated_unix_ms=excluded.updated_unix_ms")
            .bind(&key)
            .bind(value.kind())
            .bind(&serialized)
            .bind(now())
            .execute(&mut *tx)
            .await
            .map_err(|_| StoreError::Unavailable)?;
        tx.commit().await.map_err(|_| StoreError::Unavailable)?;
        self.secure_files()?;
        Ok(())
    }

    async fn get_preference(
        &self,
        scope: &PreferenceScope,
        kind: &str,
    ) -> Result<Option<PreferenceValue>, StoreError> {
        let key = scope_key(scope)?;
        let mut tx = self
            .pool()
            .await?
            .begin()
            .await
            .map_err(|_| StoreError::Unavailable)?;
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT value_json FROM user_preferences WHERE scope_key=? AND preference_kind=?",
        )
        .bind(&key)
        .bind(kind)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| StoreError::Unavailable)?;
        tx.commit().await.map_err(|_| StoreError::Unavailable)?;
        let Some((serialized,)) = row else {
            return Ok(None);
        };
        Ok(Some(decode(&serialized)?))
    }

    async fn unset_preference(
        &self,
        scope: &PreferenceScope,
        kind: &str,
    ) -> Result<(), StoreError> {
        let key = scope_key(scope)?;
        let mut tx = self
            .pool()
            .await?
            .begin()
            .await
            .map_err(|_| StoreError::Unavailable)?;
        sqlx::query("DELETE FROM user_preferences WHERE scope_key=? AND preference_kind=?")
            .bind(&key)
            .bind(kind)
            .execute(&mut *tx)
            .await
            .map_err(|_| StoreError::Unavailable)?;
        tx.commit().await.map_err(|_| StoreError::Unavailable)?;
        self.secure_files()?;
        Ok(())
    }

    async fn list_preferences(
        &self,
        scope: &PreferenceScope,
    ) -> Result<Vec<(String, PreferenceValue)>, StoreError> {
        let key = scope_key(scope)?;
        let mut tx = self
            .pool()
            .await?
            .begin()
            .await
            .map_err(|_| StoreError::Unavailable)?;
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT preference_kind, value_json FROM user_preferences WHERE scope_key=? ORDER BY preference_kind")
                .bind(&key)
                .fetch_all(&mut *tx)
                .await
                .map_err(|_| StoreError::Unavailable)?;
        tx.commit().await.map_err(|_| StoreError::Unavailable)?;
        let mut out = Vec::with_capacity(rows.len());
        for (kind, serialized) in rows {
            // A listing skips a row that fails admission rather than erroring
            // the whole list: one bad row must not hide the rest.
            match decode(&serialized) {
                Ok(value) => out.push((kind, value)),
                Err(StoreError::Invalid) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }
}

/// Decodes a `value_json` row, refusing one whose shape is a credential, SQL,
/// or path. The typed value cannot produce such a shape, so this guards a row
/// written by direct SQL or a future variant the running build does not know.
fn decode(serialized: &str) -> Result<PreferenceValue, StoreError> {
    admission::check(serialized)?;
    if crate::redact(serialized) != serialized {
        return Err(StoreError::Invalid);
    }
    serde_json::from_str::<PreferenceValue>(serialized).map_err(|_| StoreError::Invalid)
}

/// The persisted scope key: `"global"` or the profile identity string. A
/// `PreferenceScope` variant the running build does not know (the enum is
/// `#[non_exhaustive]`) has no store representation yet, so it fails closed.
fn scope_key(scope: &PreferenceScope) -> Result<String, StoreError> {
    match scope {
        PreferenceScope::Global => Ok("global".to_string()),
        PreferenceScope::Profile(id) => Ok(id.as_str().to_string()),
        _ => Err(StoreError::Invalid),
    }
}
