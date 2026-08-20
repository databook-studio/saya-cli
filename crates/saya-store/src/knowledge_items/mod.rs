//! The `knowledge_items` repository — one current-state row per knowledge slot.
//!
//! This is the home D-1 (typed slots) and D-2 (computed staleness) were given
//! a table for. Object identity is inlined on the row rather than joined to
//! `contract_objects`, so "what does SAYA know about this profile" is one
//! query. Cardinality is enforced at the storage boundary: a partial unique
//! index refuses two single-valued rows for the same object and slot — a
//! Rust-side guard alone is a convention, and conventions are what this
//! subsystem exists to delete.
//!
//! Nothing adopts this yet. The existing `contract_*` tables and every current
//! caller keep working; a later slice switches over.

mod binding;
mod error;
mod keys;
mod reads;
mod records;
mod writes;

use async_trait::async_trait;
pub use error::KnowledgeStoreError;
pub use keys::knowledge_item_id_for;
pub use records::{KnowledgeItem, KnowledgeItemRequest, MAX_KNOWLEDGE_ITEM_BYTES};

use crate::SqliteStateStore;
use saya_types::{DatabaseObjectRef, KnowledgeState, ProfileIdentity, SchemaFingerprint};

/// The repository over the `knowledge_items` table. Insert/replace enforces
/// the slot's cardinality and the payload discipline; the two reads are each
/// one query, scoped absolutely to a profile.
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
    /// `Dismissed`, in one transaction, keeping the row as a tombstone.
    async fn forget_knowledge_item(&self, id: &str) -> Result<(), KnowledgeStoreError>;
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
    /// Every knowledge item for `profile` in one query.
    async fn knowledge_for_profile(
        &self,
        profile: &ProfileIdentity,
    ) -> Result<Vec<KnowledgeItem>, KnowledgeStoreError>;
    /// Every knowledge item for `object` in one query.
    async fn knowledge_for_object(
        &self,
        object: &DatabaseObjectRef,
    ) -> Result<Vec<KnowledgeItem>, KnowledgeStoreError>;
    /// All distinct objects that have knowledge items for `profile`.
    async fn objects_for_profile(
        &self,
        profile: &ProfileIdentity,
    ) -> Result<Vec<DatabaseObjectRef>, KnowledgeStoreError>;
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
    async fn forget_knowledge_item(&self, id: &str) -> Result<(), KnowledgeStoreError> {
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
    async fn knowledge_for_object(
        &self,
        object: &DatabaseObjectRef,
    ) -> Result<Vec<KnowledgeItem>, KnowledgeStoreError> {
        reads::read_for_object(self, object).await
    }
    async fn objects_for_profile(
        &self,
        profile: &ProfileIdentity,
    ) -> Result<Vec<DatabaseObjectRef>, KnowledgeStoreError> {
        reads::read_objects_for_profile(self, profile).await
    }
}
