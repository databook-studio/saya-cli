use super::*;

#[tokio::test]
async fn contract_read_returns_one_contract_and_rejects_malformed_table() {
    let root = temp_root("read_one");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let obj = object_ref(&identity, "orders");
    // `contract_read` calls `show`, which Chunk 3 migrated to `knowledge_items`
    // — so seed an Active item there (a `Table` binding is valid against the
    // `id`-only schema `put_knowledge_item` caches).
    put_knowledge_item(
        &store,
        &obj,
        ClaimPayload::table_description("sales fact table").unwrap(),
        KnowledgeState::Active,
    )
    .await;

    let tools = DatabaseTools::with_registry(
        registry_with_primary("primary", &identity),
        100,
        true,
        Some(store.clone()),
    );

    let res = tools
        .execute(
            "contract_read",
            serde_json::json!({"table": "catalog.public.orders"}),
        )
        .await
        .expect("contract_read should succeed");
    let contract = res.get("contract").expect("one contract");
    assert_eq!(contract["object"], "catalog.public.orders");
    assert!(
        contract["claims"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["kind"] == "table_description" && c["value"] == "sales fact table"),
        "stored description should appear: {res}"
    );

    // A malformed table is a typed tool error, not a panic or an untyped string.
    // `ToolError` (in saya-agent) has no variant that
    // names the expected form, so the form is named in the tool description the
    // model reads; the typed error is `InvalidQueryArguments` (SPEC REVIEW 2b-3a).
    let err = tools
        .execute("contract_read", serde_json::json!({"table": "orders"}))
        .await
        .expect_err("malformed table should be a typed error");
    assert!(
        matches!(err, ToolError::InvalidQueryArguments),
        "malformed table should be InvalidQueryArguments: {err}"
    );

    let defs = DatabaseTools::definitions(true, true, false, false, true);
    let read_def = defs
        .iter()
        .find(|t| t.name == "contract_read")
        .expect("contract_read is defined");
    assert!(
        read_def.description.contains("catalog.schema"),
        "the description names the expected form: {}",
        read_def.description
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn contract_read_on_unknown_object_returns_empty_with_reason() {
    let root = temp_root("read_unknown");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let tools = DatabaseTools::with_registry(
        registry_with_primary("primary", &identity),
        100,
        true,
        Some(store),
    );

    let res = tools
        .execute(
            "contract_read",
            serde_json::json!({"table": "catalog.public.missing"}),
        )
        .await
        .expect("no contract is an empty result, not an error");
    assert!(res.get("contract").is_none(), "no contract object: {res}");
    let reason = res
        .get("reason")
        .and_then(|v| v.as_str())
        .expect("a short reason explains the empty result");
    assert!(!reason.is_empty());

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn contract_read_on_a_stale_object_reports_stale_with_no_claims() {
    let root = temp_root("read_stale");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let obj = object_ref(&identity, "orders");
    // An Active `default_time_column` on `created_at`. `put_knowledge_item`
    // caches an `id`-only schema (no `created_at`), so the item's
    // `Column { created_at, Time }` binding reads `Invalid` → the contract
    // aggregates to `Stale`, and the model-facing `contract_read` (`show` with
    // `ForModel`) reports the object stale with no claims — the gone-column
    // fact is not handed to the model as current.
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
        .execute(
            "contract_read",
            serde_json::json!({"table": "catalog.public.orders"}),
        )
        .await
        .expect("contract_read should succeed");
    // The object is reported as stale — the model learns the object is stale,
    // not that no contract exists (which would be the same bug in a different
    // hat).
    let contract = res.get("contract").expect("contract object present: {res}");
    assert_eq!(
        contract["schema_state"], "stale",
        "stale object is reported stale: {res}"
    );
    assert_eq!(
        contract["object"], "catalog.public.orders",
        "the stale object is named: {res}"
    );
    // And there are no claims to act on — the gone-column claim is not handed
    // to the model as a current fact.
    let claims = contract
        .get("claims")
        .and_then(|v| v.as_array())
        .expect("claims array present");
    assert!(
        claims.is_empty(),
        "no claims to act on for a stale object: {res}"
    );
    let text = serde_json::to_string(&res).unwrap();
    assert!(
        !text.contains("created_at"),
        "stale claim text must not leak: {text}"
    );

    let _ = fs::remove_dir_all(root);
}
