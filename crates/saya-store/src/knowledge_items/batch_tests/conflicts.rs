//! Conflict reporting: an active slot held by a different value is reported,
//! the rest of the batch still lands, and a forgotten row is never
//! resurrected.

use super::Fixture;
use crate::knowledge_item_id_for;
use crate::knowledge_items::{BatchItemOutcome, KnowledgeItemStore};
use saya_types::{ClaimPayload, KnowledgeSlot, KnowledgeState};

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
