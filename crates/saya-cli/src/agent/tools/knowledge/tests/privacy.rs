use super::*;

#[tokio::test]
async fn no_opaque_profile_identity_in_any_tool_result() {
    let root = temp_root("no_identity");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let obj = object_ref(&identity, "orders");
    // Both `contract_read` (via `show`) and `contract_search` (via `recall`)
    // read `knowledge_items` now; seed one Active alias so each tool sees it
    // and the identity-leak assertion covers both.
    put_knowledge_item(
        &store,
        &obj,
        ClaimPayload::table_alias("orders").unwrap(),
        KnowledgeState::Active,
    )
    .await;

    let tools = DatabaseTools::with_registry(
        registry_with_primary("primary", &identity),
        100,
        true,
        Some(store),
    );

    let search = tools
        .execute("contract_search", serde_json::json!({"terms": ["orders"]}))
        .await
        .unwrap();
    let read = tools
        .execute(
            "contract_read",
            serde_json::json!({"table": "catalog.public.orders"}),
        )
        .await
        .unwrap();

    let identity_str = identity.as_str();
    assert_eq!(identity_str.len(), 66);
    assert!(identity_str.starts_with("p-"));
    let search_text = serde_json::to_string(&search).unwrap();
    let read_text = serde_json::to_string(&read).unwrap();
    assert!(
        !search_text.contains(identity_str),
        "identity leaked into contract_search: {search_text}"
    );
    assert!(
        !read_text.contains(identity_str),
        "identity leaked into contract_read: {read_text}"
    );
    // The profile NAME the model may see is present, not the identity.
    assert!(search_text.contains("\"profile\":\"primary\""));
    assert!(read_text.contains("\"profile\":\"primary\""));

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn privacy_gate_closed_returns_empty_without_claims() {
    let root = temp_root("privacy_closed");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let obj = object_ref(&identity, "orders");
    put_knowledge_item(
        &store,
        &obj,
        ClaimPayload::table_alias("secret_alias").unwrap(),
        KnowledgeState::Active,
    )
    .await;

    // allow_query_data = false models the closed privacy gate.
    let tools = DatabaseTools::with_registry(
        registry_with_primary("primary", &identity),
        100,
        false,
        Some(store),
    );

    let res = tools
        .execute(
            "contract_search",
            serde_json::json!({"terms": ["secret_alias"]}),
        )
        .await
        .expect("privacy gate returns empty, not an error");
    assert!(
        res.get("contracts")
            .map(|c| c.as_array().unwrap().is_empty())
            .unwrap_or(true),
        "no contracts when the gate is closed: {res}"
    );
    let text = serde_json::to_string(&res).unwrap();
    assert!(!text.contains("secret_alias"));

    let res = tools
        .execute(
            "contract_read",
            serde_json::json!({"table": "catalog.public.orders"}),
        )
        .await
        .expect("privacy gate returns empty, not an error");
    assert!(
        res.get("contract").is_none(),
        "no contract when closed: {res}"
    );
    let text = serde_json::to_string(&res).unwrap();
    assert!(!text.contains("secret_alias"));

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn unopenable_store_returns_empty_result_not_error() {
    let root = temp_root("unopenable");
    fs::write(root.join("blocker"), b"x").unwrap();
    let bad = root.join("blocker/state.sqlite3");
    let store = SqliteStateStore::new(&bad);
    let identity = profile_identity("primary");

    let tools = DatabaseTools::with_registry(
        registry_with_primary("primary", &identity),
        100,
        true,
        Some(store),
    );

    let search = tools
        .execute("contract_search", serde_json::json!({"terms": ["orders"]}))
        .await
        .expect("store failure must be an empty result, not Err");
    assert!(
        search
            .get("contracts")
            .map(|c| c.as_array().unwrap().is_empty())
            .unwrap_or(true),
        "empty contracts on store failure: {search}"
    );

    let read = tools
        .execute(
            "contract_read",
            serde_json::json!({"table": "catalog.public.orders"}),
        )
        .await
        .expect("store failure must be an empty result, not Err");
    assert!(
        read.get("contract").is_none(),
        "no contract on store failure: {read}"
    );

    let _ = fs::remove_dir_all(root);
}
