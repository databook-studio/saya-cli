/// A confirmed description with the largest text a claim allows. The rendered
/// line is `table_description  <text>` — over a KiB — so a budget of a few hundred
/// bytes admits no such claim. Used by the byte-budget tests below.
async fn seed_large_description(
    store: &SqliteStateStore,
    identity: &ProfileIdentity,
    object_name: &str,
    text: &str,
) -> DatabaseObjectRef {
    let obj = object(identity, object_name);
    let tree = orders_schema_named(identity, object_name);
    store
        .upsert_schema(identity.as_str(), &tree.1)
        .await
        .unwrap();
    put_item(
        store,
        &obj,
        ClaimPayload::table_description(text).unwrap(),
        KnowledgeState::Active,
    )
    .await;
    obj
}
