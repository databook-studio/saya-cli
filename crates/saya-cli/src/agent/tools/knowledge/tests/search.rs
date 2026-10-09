use super::*;

#[tokio::test]
async fn contract_search_returns_confirmed_and_hides_candidates() {
    let root = temp_root("search_confirmed");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let obj = object_ref(&identity, "orders");
    put_knowledge_item(
        &store,
        &obj,
        ClaimPayload::table_alias("orders").unwrap(),
        KnowledgeState::Active,
    )
    .await;
    put_knowledge_item(
        &store,
        &obj,
        ClaimPayload::table_alias("secret_candidate").unwrap(),
        KnowledgeState::Pending,
    )
    .await;

    let tools = DatabaseTools::with_registry(
        registry_with_primary("primary", &identity),
        100,
        true,
        Some(store),
    );
    let res = tools
        .execute("contract_search", serde_json::json!({"terms": ["orders"]}))
        .await
        .expect("contract_search should succeed");

    let contracts = res
        .get("contracts")
        .and_then(|v| v.as_array())
        .expect("result carries a `contracts` array");
    assert_eq!(contracts.len(), 1, "one matched object");
    let claims = contracts[0]["claims"].as_array().expect("claims array");
    assert!(
        claims
            .iter()
            .any(|c| c["kind"] == "table_alias" && c["value"] == "orders"),
        "confirmed alias should appear: {res}"
    );
    assert!(
        !claims.iter().any(|c| c["value"] == "secret_candidate"),
        "candidate alias must not appear: {res}"
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn contract_search_drops_a_stale_claim_and_says_so() {
    let root = temp_root("search_stale");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let obj = object_ref(&identity, "orders");
    // A confirmed `default_time_column` item on `created_at`, filed against a
    // cached schema that has since dropped `created_at`. The D-4 binding
    // (`Column { created_at, Time }`) is no longer satisfied, so the contract
    // aggregates to `Invalid` (→ `Stale`) and the model-facing `contract_search`
    // drops it and says why. `contract_search` reads knowledge_items.
    let drifted = SchemaTree {
        databases: vec![Database {
            name: obj.catalog().to_string(),
            schemas: vec![Schema {
                name: obj.schema().to_string(),
                tables: vec![Table {
                    name: obj.object().to_string(),
                    primary_key: vec![],
                    foreign_keys: vec![],
                    columns: vec![Column {
                        name: "id".into(),
                        data_type: "bigint".into(),
                        nullable: false,
                    }],
                }],
            }],
        }],
    };
    store
        .upsert_schema(obj.profile().as_str(), &drifted)
        .await
        .unwrap();
    put_knowledge_item(
        &store,
        &obj,
        ClaimPayload::default_time_column("created_at", None).unwrap(),
        KnowledgeState::Active,
    )
    .await;

    let tools = DatabaseTools::with_registry(
        registry_with_primary("primary", &identity),
        100,
        true,
        Some(store),
    );
    let res = tools
        .execute("contract_search", serde_json::json!({"terms": ["orders"]}))
        .await
        .expect("contract_search should succeed");
    // The stale contract is dropped for the model — no contracts array entries.
    let contracts = res
        .get("contracts")
        .and_then(|v| v.as_array())
        .expect("result carries a `contracts` array");
    assert!(
        contracts.is_empty(),
        "stale contract must not appear: {res}"
    );
    // The result says *why* it is empty, so the model does not retry the same
    // terms expecting a different answer.
    let reason = res
        .get("reason")
        .and_then(|v| v.as_str())
        .expect("a stale exclusion carries a reason");
    assert!(
        reason.contains("stale"),
        "reason names staleness, got: {reason}"
    );
    // The gone-column claim value never reaches the result.
    let text = serde_json::to_string(&res).unwrap();
    assert!(
        !text.contains("created_at"),
        "stale claim text must not leak: {text}"
    );

    let _ = fs::remove_dir_all(root);
}
