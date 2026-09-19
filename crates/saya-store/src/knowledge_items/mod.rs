//! The `knowledge_items` repository — one current-state row per knowledge slot.
//!
//! This is the home D-1 (typed slots) and D-2 (computed staleness) were given
//! a table for. Object identity is inlined on the row rather than joined to
//! `contract_objects`, so "what does SAYA know about this profile" needs no
//! join. Cardinality is enforced at the storage boundary: a partial unique
//! index refuses two single-valued rows for the same object and slot — a
//! Rust-side guard alone is a convention, and conventions are what this
//! subsystem exists to delete.
//!
//! Recall, review, queue, and learning callers use this repository; the
//! compatibility `Vec` reads remain for small administrative/test consumers.

mod binding;
mod error;
mod keys;
mod pagination;
mod reads;
mod records;
mod writes;

use async_trait::async_trait;
pub use error::KnowledgeStoreError;
pub use keys::knowledge_item_id_for;
pub use pagination::{
    KnowledgeCursor, KnowledgeItemsQuery, KnowledgeObjectsQuery, KnowledgePage,
    MAX_KNOWLEDGE_PAGE_SIZE,
};
pub use records::{
    CleanupState, KnowledgeItem, KnowledgeItemRequest, MAX_KNOWLEDGE_ITEM_BYTES,
    MAX_SCHEMA_BINDING_BYTES,
};

use crate::SqliteStateStore;
use saya_types::{DatabaseObjectRef, KnowledgeState, ProfileIdentity, SchemaFingerprint};
use sqlx::SqlitePool;
use std::path::Path;

/// The logical forget commit succeeded; physical byte cleanup may need a retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForgetOutcome {
    Cleaned,
    CleanupPending,
}

/// The repository over the `knowledge_items` table. Insert/replace enforces
/// the slot's cardinality and the payload discipline; paged reads are scoped
/// absolutely to a profile.
#[async_trait]
pub trait KnowledgeItemStore: Send + Sync {
    /// Insert a knowledge item, replacing the one row a single-valued slot
    /// admits, or appending a distinct value to a multi-valued slot (refused
    /// past its declared bound). Refuses a payload that does not match the
    /// slot, an oversized value, or a value that structurally resembles a
    /// credential or raw SQL.
    async fn put_knowledge_item(
        &self,
        request: KnowledgeItemRequest,
    ) -> Result<(), KnowledgeStoreError>;
    /// Retrieve a single knowledge item by its unique ID.
    async fn get_knowledge_item(
        &self,
        id: &str,
    ) -> Result<Option<KnowledgeItem>, KnowledgeStoreError>;
    /// Update the state of an existing knowledge item.
    async fn update_knowledge_item_state(
        &self,
        id: &str,
        state: KnowledgeState,
    ) -> Result<(), KnowledgeStoreError>;
    /// Forget an item: erase its value and schema binding and mark it
    /// `Dismissed`, in one transaction, keeping the row as a tombstone. The
    /// outcome says whether post-commit physical cleanup completed or is
    /// pending a retry.
    async fn forget_knowledge_item(&self, id: &str) -> Result<ForgetOutcome, KnowledgeStoreError>;
    /// Revalidate an item, updating its schema binding JSON, fingerprint version,
    /// and transitioning its state to `Active`.
    async fn revalidate_knowledge_item(
        &self,
        id: &str,
        fingerprint: SchemaFingerprint,
        schema_binding_json: String,
    ) -> Result<(), KnowledgeStoreError>;
    /// Delete a knowledge item by its unique ID.
    async fn delete_knowledge_item(&self, id: &str) -> Result<(), KnowledgeStoreError>;
    /// Every knowledge item for `profile`, collected through bounded pages.
    async fn knowledge_for_profile(
        &self,
        profile: &ProfileIdentity,
    ) -> Result<Vec<KnowledgeItem>, KnowledgeStoreError>;
    /// Retrieve one bounded, deterministic page for `profile`.
    async fn knowledge_for_profile_page(
        &self,
        profile: &ProfileIdentity,
        query: KnowledgeItemsQuery,
    ) -> Result<KnowledgePage<KnowledgeItem>, KnowledgeStoreError>;
    /// Every knowledge item for `object`, collected through bounded pages.
    async fn knowledge_for_object(
        &self,
        object: &DatabaseObjectRef,
    ) -> Result<Vec<KnowledgeItem>, KnowledgeStoreError>;
    /// Retrieve one bounded, deterministic page for `object`.
    async fn knowledge_for_object_page(
        &self,
        object: &DatabaseObjectRef,
        query: KnowledgeItemsQuery,
    ) -> Result<KnowledgePage<KnowledgeItem>, KnowledgeStoreError>;
    /// All distinct objects that have knowledge items for `profile`.
    async fn objects_for_profile(
        &self,
        profile: &ProfileIdentity,
    ) -> Result<Vec<DatabaseObjectRef>, KnowledgeStoreError>;
    /// Retrieve one bounded, deterministic page of distinct profile objects.
    async fn objects_for_profile_page(
        &self,
        profile: &ProfileIdentity,
        query: KnowledgeObjectsQuery,
    ) -> Result<KnowledgePage<DatabaseObjectRef>, KnowledgeStoreError>;
}

#[async_trait]
impl KnowledgeItemStore for SqliteStateStore {
    async fn put_knowledge_item(
        &self,
        request: KnowledgeItemRequest,
    ) -> Result<(), KnowledgeStoreError> {
        writes::insert_or_replace(self, &request).await
    }
    async fn get_knowledge_item(
        &self,
        id: &str,
    ) -> Result<Option<KnowledgeItem>, KnowledgeStoreError> {
        reads::read_by_id(self, id).await
    }
    async fn update_knowledge_item_state(
        &self,
        id: &str,
        state: KnowledgeState,
    ) -> Result<(), KnowledgeStoreError> {
        writes::update_state(self, id, state).await
    }
    async fn forget_knowledge_item(&self, id: &str) -> Result<ForgetOutcome, KnowledgeStoreError> {
        writes::forget_item(self, id).await
    }
    async fn revalidate_knowledge_item(
        &self,
        id: &str,
        fingerprint: SchemaFingerprint,
        schema_binding_json: String,
    ) -> Result<(), KnowledgeStoreError> {
        writes::revalidate_item(self, id, fingerprint, schema_binding_json).await
    }
    async fn delete_knowledge_item(&self, id: &str) -> Result<(), KnowledgeStoreError> {
        writes::delete_item(self, id).await
    }
    async fn knowledge_for_profile(
        &self,
        profile: &ProfileIdentity,
    ) -> Result<Vec<KnowledgeItem>, KnowledgeStoreError> {
        reads::read_for_profile(self, profile).await
    }
    async fn knowledge_for_profile_page(
        &self,
        profile: &ProfileIdentity,
        query: KnowledgeItemsQuery,
    ) -> Result<KnowledgePage<KnowledgeItem>, KnowledgeStoreError> {
        reads::read_for_profile_page(self, profile, &query).await
    }
    async fn knowledge_for_object(
        &self,
        object: &DatabaseObjectRef,
    ) -> Result<Vec<KnowledgeItem>, KnowledgeStoreError> {
        reads::read_for_object(self, object).await
    }
    async fn knowledge_for_object_page(
        &self,
        object: &DatabaseObjectRef,
        query: KnowledgeItemsQuery,
    ) -> Result<KnowledgePage<KnowledgeItem>, KnowledgeStoreError> {
        reads::read_for_object_page(self, object, &query).await
    }
    async fn objects_for_profile(
        &self,
        profile: &ProfileIdentity,
    ) -> Result<Vec<DatabaseObjectRef>, KnowledgeStoreError> {
        reads::read_objects_for_profile(self, profile).await
    }
    async fn objects_for_profile_page(
        &self,
        profile: &ProfileIdentity,
        query: KnowledgeObjectsQuery,
    ) -> Result<KnowledgePage<DatabaseObjectRef>, KnowledgeStoreError> {
        reads::read_objects_for_profile_page(self, profile, &query).await
    }
}

/// Best-effort recovery for tombstones whose logical commit preceded a failed
/// WAL checkpoint. Opening remains available when the filesystem is still
/// unhealthy; the pending marker makes the next open or explicit retry safe.
pub(crate) async fn recover_pending_cleanup(pool: &SqlitePool, path: &Path) {
    let pending: Result<i64, _> =
        sqlx::query_scalar("SELECT COUNT(*) FROM knowledge_items WHERE cleanup_state='pending'")
            .fetch_one(pool)
            .await;
    if !matches!(pending, Ok(value) if value > 0) {
        return;
    }
    if sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(pool)
        .await
        .is_err()
        || crate::sqlite_support::secure_files(path).is_err()
    {
        return;
    }
    let _ = sqlx::query(
        "UPDATE knowledge_items SET cleanup_state='complete' WHERE cleanup_state='pending'",
    )
    .execute(pool)
    .await;
}
