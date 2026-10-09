use super::*;

#[tokio::test]
async fn contract_read_truncates_claims_past_the_bound_and_says_so() {
    let root = temp_root("read_truncated");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let obj = object_ref(&identity, "wide");
    // More claims than the 12-per-object `read_payload` bound. D-4 caps a
    // multi-valued slot at 4 values, so seed `column_description` items across
    // five columns (4 each = 20): each is a distinct `Column { colN, Exists }`
    // slot, so cardinality admits four per column. `put_knowledge_item` caches
    // an `id`-only schema per call; upsert a five-column schema once at the end
    // so every binding reads `Valid` (Current) and `show` returns all 20.
    for col in 0..5 {
        for n in 0..4 {
            put_knowledge_item(
                &store,
                &obj,
                ClaimPayload::column_description(format!("c{col}"), format!("desc {n}")).unwrap(),
                KnowledgeState::Active,
            )
            .await;
        }
    }
    let wide = SchemaTree {
        databases: vec![Database {
            name: obj.catalog().to_string(),
            schemas: vec![Schema {
                name: obj.schema().to_string(),
                tables: vec![Table {
                    name: obj.object().to_string(),
                    primary_key: vec![],
                    foreign_keys: vec![],
                    columns: (0..5)
                        .map(|col| Column {
                            name: format!("c{col}"),
                            data_type: "text".into(),
                            nullable: true,
                        })
                        .collect(),
                }],
            }],
        }],
    };
    store
        .upsert_schema(obj.profile().as_str(), &wide)
        .await
        .unwrap();

    let tools = DatabaseTools::with_registry(
        registry_with_primary("primary", &identity),
        100,
        true,
        Some(store),
    );
    let res = tools
        .execute(
            "contract_read",
            serde_json::json!({"table": "catalog.public.wide"}),
        )
        .await
        .expect("contract_read should succeed");
    let contract = res.get("contract").expect("one contract");
    let claims = contract["claims"].as_array().expect("claims array");
    assert!(
        claims.len() <= 12,
        "claims bounded to 12, got {}: {res}",
        claims.len()
    );
    assert!(
        contract["truncated"] == serde_json::Value::Bool(true),
        "truncation must be flagged explicitly: {res}"
    );

    let _ = fs::remove_dir_all(root);
}
