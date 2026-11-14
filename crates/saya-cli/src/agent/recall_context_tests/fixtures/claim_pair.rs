/// Seeds the orders table's cached schema and two confirmed claims on it: a
/// `default_time_column` and a `table_alias`. Returns both claim ids in the
/// order they were stored, so a test can assert the receipt names exactly them.
async fn seed_two_confirmed_claims(
    store: &SqliteStateStore,
    identity: &ProfileIdentity,
) -> (DatabaseObjectRef, ClaimId, ClaimId) {
    use saya_store::KnowledgeItemStore;
    let obj = object(identity, "orders");
    let tree = orders_schema(identity);
    store
        .upsert_schema(identity.as_str(), &tree.1)
        .await
        .unwrap();
    put_item(
        store,
        &obj,
        ClaimPayload::default_time_column("created_at", None).unwrap(),
        KnowledgeState::Active,
    )
    .await;
    put_item(
        store,
        &obj,
        ClaimPayload::table_alias("orders_alias").unwrap(),
        KnowledgeState::Active,
    )
    .await;
    // Read the store-assigned `ki-` ids back so the receipt test can compare
    // against exactly what was supplied, matching each row by its slot.
    let time_id = ClaimId::parse(
        &store
            .knowledge_for_object(&obj)
            .await
            .expect("knowledge items listed")
            .into_iter()
            .find(|i| i.slot == KnowledgeSlot::TableDefaultTime)
            .expect("time item stored")
            .id,
    )
    .expect("ki id");
    let alias_id = ClaimId::parse(
        &store
            .knowledge_for_object(&obj)
            .await
            .expect("knowledge items listed")
            .into_iter()
            .find(|i| i.slot == KnowledgeSlot::TableAlias)
            .expect("alias item stored")
            .id,
    )
    .expect("ki id");
    (obj, time_id, alias_id)
}
