/// A confirmed claim and a candidate claim on the same object, under matching
/// schema. Used by the include-candidates / confirmed tests below.
async fn seed_orders_confirmed_and_candidate(
    store: &SqliteStateStore,
    identity: &ProfileIdentity,
) -> (DatabaseObjectRef, saya_types::SchemaFingerprint) {
    let obj = object(identity, "orders");
    let tree = orders_schema(identity);
    store
        .upsert_schema(identity.as_str(), &tree.1)
        .await
        .unwrap();
    let fp = live_fingerprint(&orders_table());
    // Confirmed: the alias "orders" — recallable today.
    remember_confirmed_alias(store, &obj, &fp, "orders").await;
    // Candidate: a default time column the assistant inferred but no human
    // confirmed. Under `confirmed` it is excluded; under `include-candidates`
    // it is admitted and must be labelled unconfirmed.
    remember_candidate_default_time_column(store, &obj, &fp, "created_at").await;
    (obj, fp)
}

async fn remember_confirmed_alias(
    store: &SqliteStateStore,
    obj: &DatabaseObjectRef,
    _fingerprint: &saya_types::SchemaFingerprint,
    alias: &str,
) {
    put_item(
        store,
        obj,
        ClaimPayload::table_alias(alias).unwrap(),
        KnowledgeState::Active,
    )
    .await;
}
