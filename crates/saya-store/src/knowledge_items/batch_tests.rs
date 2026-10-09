//! Tests for the transactional batch insert (`apply_pending_batch`).

use super::{KnowledgeItemRequest, NewKnowledgeItem};
use crate::SqliteStateStore;
use saya_types::{
    ClaimOrigin, ClaimPayload, DatabaseObjectKind, DatabaseObjectRef, KnowledgeSlot,
    KnowledgeState, ProfileIdentity, SchemaFingerprint,
};
use std::{fs, path::PathBuf, time::SystemTime};

#[path = "batch_tests/bounds.rs"]
mod bounds;
#[path = "batch_tests/conflicts.rs"]
mod conflicts;
#[path = "batch_tests/landing.rs"]
mod landing;
#[path = "batch_tests/validation.rs"]
mod validation;

/// A throwaway store with one profile and helpers to build batch items. The
/// directory is removed on drop.
struct Fixture {
    store: SqliteStateStore,
    root: PathBuf,
    profile: ProfileIdentity,
}

impl Fixture {
    fn open(tag: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("saya-batch-{tag}-{stamp}"));
        fs::create_dir_all(&root).unwrap();
        let store = SqliteStateStore::new(root.join("state.sqlite3"));
        let profile = ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap();
        Self {
            store,
            root,
            profile,
        }
    }

    fn object(&self, name: &str) -> DatabaseObjectRef {
        DatabaseObjectRef::new(
            self.profile.clone(),
            "db",
            "public",
            name,
            DatabaseObjectKind::Table,
        )
        .unwrap()
    }

    fn item(
        &self,
        object: &DatabaseObjectRef,
        slot: KnowledgeSlot,
        value: ClaimPayload,
    ) -> NewKnowledgeItem {
        NewKnowledgeItem {
            object: object.clone(),
            slot,
            value,
            source: ClaimOrigin::TeamFile,
            schema_binding_json: r#"{"type":"table"}"#.into(),
            fingerprint: SchemaFingerprint::from_parts(1, &"a".repeat(64)).unwrap(),
        }
    }

    fn request(
        &self,
        object: &DatabaseObjectRef,
        slot: KnowledgeSlot,
        value: ClaimPayload,
        state: KnowledgeState,
    ) -> KnowledgeItemRequest {
        KnowledgeItemRequest {
            object: object.clone(),
            slot,
            value,
            source: ClaimOrigin::UserExplicit,
            state,
            schema_binding_json: r#"{"type":"table"}"#.into(),
            fingerprint: SchemaFingerprint::from_parts(1, &"a".repeat(64)).unwrap(),
        }
    }

    async fn row_count(&self) -> i64 {
        let pool = self.store.pool().await.unwrap();
        sqlx::query_scalar("SELECT COUNT(*) FROM knowledge_items")
            .fetch_one(pool)
            .await
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
