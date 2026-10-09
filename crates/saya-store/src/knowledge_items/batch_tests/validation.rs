//! Validation and admission: an item that does not match its slot refuses
//! the batch before it opens, and `validate_new_item` refuses what the batch
//! refuses — including credential- and SQL-shaped descriptions.

use super::Fixture;
use crate::StoreError;
use crate::knowledge_items::{KnowledgeItemStore, KnowledgeStoreError, validate_new_item};
use saya_types::{ClaimPayload, KnowledgeSlot};

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

#[tokio::test]
async fn validate_new_item_refuses_what_the_batch_refuses() {
    let fx = Fixture::open("validate");
    let orders = fx.object("orders");
    let good = fx.item(
        &orders,
        KnowledgeSlot::TableDescription,
        ClaimPayload::table_description("One row per confirmed order.").unwrap(),
    );
    assert!(matches!(validate_new_item(&good), Ok(())));
    for text in [
        "Select the latest row from each customer.",
        "The export lives in /Users/someone/project.",
        "The service sends authorization: Bearer xyz.",
        "-----BEGIN PRIVATE KEY-----abc",
    ] {
        let bad = fx.item(
            &orders,
            KnowledgeSlot::TableDescription,
            ClaimPayload::table_description(text).unwrap(),
        );
        assert!(validate_new_item(&bad).is_err(), "should refuse {text:?}");
        let result = fx.store.apply_pending_batch(&[bad]).await;
        assert!(
            matches!(result, Err(KnowledgeStoreError::Store(StoreError::Invalid))),
            "the batch refuses {text:?} too"
        );
    }
    assert_eq!(fx.row_count().await, 0);
}
