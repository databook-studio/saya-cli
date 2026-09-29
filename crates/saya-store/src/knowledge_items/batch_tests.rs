//! Tests for the transactional batch insert (`apply_pending_batch`).

use super::{
    BatchItemOutcome, KnowledgeItemRequest, KnowledgeItemStore, KnowledgeStoreError,
    MAX_PENDING_BATCH_ITEMS, NewKnowledgeItem,
};
use crate::{SqliteStateStore, StoreError, knowledge_item_id_for};
use saya_types::{
    ClaimOrigin, ClaimPayload, DatabaseObjectKind, DatabaseObjectRef, KnowledgeSlot,
    KnowledgeState, ProfileIdentity, SchemaFingerprint,
};
use std::{fs, path::PathBuf, time::SystemTime};

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

#[tokio::test]
async fn new_batch_lands_pending_rows() {
    let fx = Fixture::open("pending");
    let orders = fx.object("orders");
    let items = vec![
        fx.item(
            &orders,
            KnowledgeSlot::TableDescription,
            ClaimPayload::table_description("Orders from the ERP.").unwrap(),
        ),
        fx.item(
            &orders,
            KnowledgeSlot::TableGrain,
            ClaimPayload::table_grain("One row per order line.", None).unwrap(),
        ),
    ];
    let outcome = fx.store.apply_pending_batch(&items).await.unwrap();
    let expected = knowledge_item_id_for(
        &orders,
        &KnowledgeSlot::TableDescription,
        &serde_json::to_string(&items[0].value).unwrap(),
    );
    assert_eq!(outcome.items.len(), 2);
    assert_eq!(
        outcome.items[0],
        BatchItemOutcome::Inserted { id: expected }
    );
    let rows = fx.store.knowledge_for_object(&orders).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .all(|row| row.state == KnowledgeState::Pending && row.source == ClaimOrigin::TeamFile)
    );
}

#[tokio::test]
async fn reapplying_the_same_batch_skips_every_item() {
    let fx = Fixture::open("idempotent");
    let orders = fx.object("orders");
    let items = vec![
        fx.item(
            &orders,
            KnowledgeSlot::TableGrain,
            ClaimPayload::table_grain("One row per order line.", None).unwrap(),
        ),
        fx.item(
            &orders,
            KnowledgeSlot::TableUserNote,
            ClaimPayload::table_user_note("Fact.").unwrap(),
        ),
    ];
    fx.store.apply_pending_batch(&items).await.unwrap();
    let second = fx.store.apply_pending_batch(&items).await.unwrap();
    assert!(
        second
            .items
            .iter()
            .all(|item| *item == BatchItemOutcome::Skipped)
    );
    let rows = fx.store.knowledge_for_object(&orders).await.unwrap();
    assert_eq!(rows.len(), 2);
    // A skipped item wrote nothing, so no row was re-stamped.
    assert!(
        rows.iter()
            .all(|row| row.created_unix_ms == row.updated_unix_ms)
    );
}

#[tokio::test]
async fn conflicting_active_slot_is_reported_and_the_rest_still_lands() {
    let fx = Fixture::open("conflict");
    let orders = fx.object("orders");
    let customers = fx.object("customers");
    let local = ClaimPayload::table_grain("One row per order line.", None).unwrap();
    fx.store
        .put_knowledge_item(fx.request(
            &orders,
            KnowledgeSlot::TableGrain,
            local.clone(),
            KnowledgeState::Active,
        ))
        .await
        .unwrap();
    let local_id = knowledge_item_id_for(
        &orders,
        &KnowledgeSlot::TableGrain,
        &serde_json::to_string(&local).unwrap(),
    );
    let items = vec![
        fx.item(
            &orders,
            KnowledgeSlot::TableGrain,
            ClaimPayload::table_grain("One row per customer.", None).unwrap(),
        ),
        fx.item(
            &customers,
            KnowledgeSlot::TableAlias,
            ClaimPayload::table_alias("crm_customers").unwrap(),
        ),
    ];
    let outcome = fx.store.apply_pending_batch(&items).await.unwrap();
    assert_eq!(
        outcome.items[0],
        BatchItemOutcome::Conflict {
            existing_id: local_id.clone()
        }
    );
    assert!(matches!(
        outcome.items[1],
        BatchItemOutcome::Inserted { .. }
    ));
    // The local Active row is untouched: same value, same state.
    let row = fx
        .store
        .get_knowledge_item(&local_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.state, KnowledgeState::Active);
    assert_eq!(row.value, local);
    // The non-conflicting item of the same batch is written.
    let landed = fx.store.knowledge_for_object(&customers).await.unwrap();
    assert_eq!(landed.len(), 1);
    assert_eq!(landed[0].state, KnowledgeState::Pending);
    // The identical value itself is a skip, not a conflict.
    let same = vec![fx.item(&orders, KnowledgeSlot::TableGrain, local)];
    let outcome = fx.store.apply_pending_batch(&same).await.unwrap();
    assert_eq!(outcome.items[0], BatchItemOutcome::Skipped);
}

#[tokio::test]
async fn a_forgotten_row_is_a_conflict_and_is_never_resurrected() {
    let fx = Fixture::open("forgotten");
    let orders = fx.object("orders");
    let grain = ClaimPayload::table_grain("One row per order line.", None).unwrap();
    let note = ClaimPayload::table_user_note("Caveat.").unwrap();
    fx.store
        .put_knowledge_item(fx.request(
            &orders,
            KnowledgeSlot::TableGrain,
            grain.clone(),
            KnowledgeState::Active,
        ))
        .await
        .unwrap();
    let grain_id = knowledge_item_id_for(
        &orders,
        &KnowledgeSlot::TableGrain,
        &serde_json::to_string(&grain).unwrap(),
    );
    fx.store.forget_knowledge_item(&grain_id).await.unwrap();
    fx.store
        .put_knowledge_item(fx.request(
            &orders,
            KnowledgeSlot::TableUserNote,
            note.clone(),
            KnowledgeState::Active,
        ))
        .await
        .unwrap();
    let note_id = knowledge_item_id_for(
        &orders,
        &KnowledgeSlot::TableUserNote,
        &serde_json::to_string(&note).unwrap(),
    );
    fx.store.forget_knowledge_item(&note_id).await.unwrap();

    let outcome = fx
        .store
        .apply_pending_batch(&[
            fx.item(&orders, KnowledgeSlot::TableGrain, grain),
            fx.item(&orders, KnowledgeSlot::TableUserNote, note),
        ])
        .await
        .unwrap();
    // A forgotten fact is reported, not silently skipped or revived — for
    // single-valued and multi-valued slots alike.
    assert_eq!(
        outcome.items[0],
        BatchItemOutcome::Conflict {
            existing_id: grain_id
        }
    );
    assert_eq!(
        outcome.items[1],
        BatchItemOutcome::Conflict {
            existing_id: note_id
        }
    );
    assert_eq!(fx.row_count().await, 2);
    let rows = fx.store.knowledge_for_object(&orders).await.unwrap();
    assert!(
        rows.iter()
            .all(|row| row.state == KnowledgeState::Dismissed)
    );
}

#[tokio::test]
async fn a_storage_failure_mid_batch_writes_nothing() {
    let fx = Fixture::open("rollback");
    let orders = fx.object("orders");
    // A row that bypassed the id scheme while holding the single-valued tuple
    // the second item will claim: the partial unique index refuses the insert
    // mid-batch, after the first item is already written in the transaction.
    sqlx::query("INSERT INTO knowledge_items(id, profile_id, catalog, schema, object, object_kind, slot, cardinality, value_json, source, state, schema_binding_json, cleanup_state, fingerprint_version, created_unix_ms, updated_unix_ms) VALUES ('ki-foreign', ?, 'db', 'public', 'orders', 'table', 'table.grain', 'single', '{}', 'team_file', 'active', '{}', 'complete', 1, 0, 0)")
        .bind(fx.profile.as_str())
        .execute(fx.store.pool().await.unwrap())
        .await
        .unwrap();
    let items = vec![
        fx.item(
            &orders,
            KnowledgeSlot::TableAlias,
            ClaimPayload::table_alias("erp_orders").unwrap(),
        ),
        fx.item(
            &orders,
            KnowledgeSlot::TableGrain,
            ClaimPayload::table_grain("One row per customer.", None).unwrap(),
        ),
    ];
    let result = fx.store.apply_pending_batch(&items).await;
    assert!(matches!(
        result,
        Err(KnowledgeStoreError::Store(StoreError::Unavailable))
    ));
    // The first item was rolled back together with the failed one.
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM knowledge_items WHERE id=?")
        .bind(knowledge_item_id_for(
            &orders,
            &KnowledgeSlot::TableAlias,
            &serde_json::to_string(&items[0].value).unwrap(),
        ))
        .fetch_one(fx.store.pool().await.unwrap())
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(fx.row_count().await, 1);
}

#[tokio::test]
async fn batches_over_the_cap_are_refused_before_any_write() {
    let fx = Fixture::open("cap");
    let orders = fx.object("orders");
    let items: Vec<NewKnowledgeItem> = (0..=MAX_PENDING_BATCH_ITEMS)
        .map(|index| {
            fx.item(
                &orders,
                KnowledgeSlot::TableUserNote,
                ClaimPayload::table_user_note(format!("Note {index}.")).unwrap(),
            )
        })
        .collect();
    let result = fx.store.apply_pending_batch(&items).await;
    assert!(matches!(result, Err(KnowledgeStoreError::BoundExceeded)));
    assert_eq!(fx.row_count().await, 0);
}

#[tokio::test]
async fn a_multi_slot_past_its_bound_rolls_the_batch_back() {
    let fx = Fixture::open("multibound");
    let orders = fx.object("orders");
    let customers = fx.object("customers");
    for index in 0..4 {
        fx.store
            .put_knowledge_item(fx.request(
                &orders,
                KnowledgeSlot::TableUserNote,
                ClaimPayload::table_user_note(format!("Note {index}.")).unwrap(),
                KnowledgeState::Active,
            ))
            .await
            .unwrap();
    }
    let items = vec![
        fx.item(
            &customers,
            KnowledgeSlot::TableAlias,
            ClaimPayload::table_alias("crm_customers").unwrap(),
        ),
        fx.item(
            &orders,
            KnowledgeSlot::TableUserNote,
            ClaimPayload::table_user_note("Fifth note.").unwrap(),
        ),
    ];
    let result = fx.store.apply_pending_batch(&items).await;
    assert!(matches!(result, Err(KnowledgeStoreError::BoundExceeded)));
    assert_eq!(fx.row_count().await, 4);
}

#[tokio::test]
async fn an_invalid_item_refuses_the_batch_before_it_opens() {
    let fx = Fixture::open("invalid");
    let orders = fx.object("orders");
    let items = vec![fx.item(
        &orders,
        KnowledgeSlot::TableGrain,
        ClaimPayload::table_alias("erp_orders").unwrap(),
    )];
    let result = fx.store.apply_pending_batch(&items).await;
    assert!(matches!(
        result,
        Err(KnowledgeStoreError::CardinalityMismatch)
    ));
    assert_eq!(fx.row_count().await, 0);
}
