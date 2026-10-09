/// FIX-S6a (g): the `contracts` tool serves a model host under the
/// model-facing retrieval policy: a contract whose bound column the cached
/// schema no longer has computes `Stale`, is absent from the result, and
/// the exclusion is counted — never handed to the model as a current fact.
#[tokio::test]
async fn a_stale_contract_is_absent_from_the_contracts_tool_and_counted() {
    use saya_store::{KnowledgeItemRequest, KnowledgeItemStore as _, SchemaStore as _};
    use saya_types::{
        ClaimOrigin, ClaimPayload, Column, Database, DatabaseObjectKind, DatabaseObjectRef,
        KnowledgeSlot, KnowledgeState, Schema, SchemaBinding, SchemaTree, Table,
    };
    let root = std::env::temp_dir().join(format!("saya-mcp-stale-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let runtime = fixture_runtime(
        "[profiles.local]\ntype = 'sqlite'\npath = 'a.sqlite3'\n",
        "default_profile = 'local'\n",
    );
    let store = saya_store::SqliteStateStore::new(root.join("state.sqlite3"));
    let context = std::sync::Arc::new(super::context::McpContext {
        runtime: runtime.clone(),
        store,
        replay_slot: tokio::sync::Mutex::new(()),
    });
    let policy = ServePolicy::resolve(&runtime, &["local".to_owned()], None).unwrap();
    let (_profile, identity) = context
        .allowed_profile(policy.allowlist(), "local")
        .expect("the allowlisted profile resolves");
    // The drifted cache first (it also runs the store migrations): `orders`
    // no longer has `created_at`.
    let tree = SchemaTree {
        databases: vec![Database {
            name: "catalog".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![Table {
                    name: "orders".into(),
                    columns: vec![Column {
                        name: "id".into(),
                        data_type: "bigint".into(),
                        nullable: false,
                    }],
                    primary_key: vec![],
                    foreign_keys: vec![],
                }],
            }],
        }],
    };
    context
        .store
        .upsert_schema(identity.as_str(), &tree)
        .await
        .unwrap();
    // A confirmed `default_time_column` claim whose D-4 binding is
    // `Column { created_at, Time }` — gone from the drifted cache.
    let object = DatabaseObjectRef::new(
        identity.clone(),
        "catalog",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let payload = ClaimPayload::default_time_column("created_at", None).unwrap();
    let slot = KnowledgeSlot::TableDefaultTime;
    let binding = SchemaBinding::derive(&slot, &payload).unwrap();
    context
        .store
        .put_knowledge_item(KnowledgeItemRequest {
            object: object.clone(),
            slot,
            value: payload,
            source: ClaimOrigin::UserExplicit,
            state: KnowledgeState::Active,
            schema_binding_json: serde_json::to_string(&binding).unwrap(),
            fingerprint: crate::commands::unobserved_fingerprint(),
        })
        .await
        .unwrap();
    let request: rmcp::model::CallToolRequestParams = serde_json::from_value(serde_json::json!({
        "name": "contracts",
        "arguments": {"profile": "local"},
    }))
    .unwrap();
    let response = super::context_tools::contracts(&policy, &context, &request)
        .await
        .unwrap();
    match response {
        rmcp::model::CallToolResponse::Complete(result) => {
            assert_ne!(result.is_error, Some(true), "the tool answers: {result:?}");
            let payload = result
                .structured_content
                .expect("a structured answer carries the payload");
            assert_eq!(
                payload["contracts"],
                serde_json::json!([]),
                "the stale contract is absent: {payload}"
            );
            assert_eq!(
                payload["excluded_by_schema"], 1,
                "the exclusion is counted, never silent: {payload}"
            );
        }
        other => panic!("a tool answer is a complete result, not {other:?}"),
    }
    let _ = std::fs::remove_dir_all(root);
}
