//! A batch lands every item as a `Pending` row; re-applying the identical
//! batch skips every item and leaves rows unstamped.

use super::Fixture;
use crate::knowledge_item_id_for;
use crate::knowledge_items::{BatchItemOutcome, KnowledgeItemStore};
use saya_types::{ClaimOrigin, ClaimPayload, KnowledgeSlot, KnowledgeState};

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
