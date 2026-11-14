//! Transaction and bound enforcement: a storage failure mid-batch writes
//! nothing, batches past the cap are refused before any write, and a
//! multi-valued slot past its bound rolls the batch back.

use super::Fixture;
use crate::StoreError;
use crate::knowledge_item_id_for;
use crate::knowledge_items::{
    KnowledgeItemStore, KnowledgeStoreError, MAX_PENDING_BATCH_ITEMS, NewKnowledgeItem,
};
use saya_types::{ClaimPayload, KnowledgeSlot, KnowledgeState};

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
