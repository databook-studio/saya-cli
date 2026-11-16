use super::*;
use saya_store::KnowledgeItemStore;

#[tokio::test]
async fn test_ingest_proposals_persists_to_sqlite_store() {
    let root = temp_root("persist");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);

    let obj = test_object("orders");
    let proposal = ResolvedProposal {
        object: obj.clone(),
        profile_name: "primary".to_string(),
        slot: KnowledgeSlot::TableGrain,
        value: ClaimPayload::table_grain("one row per order", None).unwrap(),
        source: ClaimOrigin::AssistantInferred,
        state: KnowledgeState::Pending,
        schema_binding: SchemaBinding::Table,
    };

    let fingerprint = SchemaFingerprint::from_parts(1, &"a".repeat(64)).unwrap();
    let dtos = ingest_proposals(&store, vec![proposal], fingerprint)
        .await
        .unwrap();

    assert_eq!(dtos.len(), 1);
    assert_eq!(dtos[0].profile, "primary");
    assert_eq!(dtos[0].object, obj.qualified_name());
    assert_eq!(dtos[0].kind, "table_grain");
    assert_eq!(dtos[0].value, "one row per order");
    assert_eq!(dtos[0].status, ClaimStatus::Candidate);

    let persisted = store.knowledge_for_object(&obj).await.unwrap();
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0].slot, KnowledgeSlot::TableGrain);
    assert_eq!(persisted[0].state, KnowledgeState::Pending);
    assert_eq!(persisted[0].source, ClaimOrigin::AssistantInferred);
}

#[tokio::test]
async fn test_ingest_proposals_returns_dtos_for_persisted_items() {
    let root = temp_root("dtos");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);

    let obj = test_object("products");
    let proposal = ResolvedProposal {
        object: obj.clone(),
        profile_name: "analytics".to_string(),
        slot: KnowledgeSlot::TableAlias,
        value: ClaimPayload::table_alias("catalog_items").unwrap(),
        source: ClaimOrigin::UserExplicit,
        state: KnowledgeState::Active,
        schema_binding: SchemaBinding::Table,
    };

    let fingerprint = SchemaFingerprint::from_parts(1, &"b".repeat(64)).unwrap();
    let dtos = ingest_proposals(&store, vec![proposal], fingerprint)
        .await
        .unwrap();

    assert_eq!(dtos.len(), 1);
    let dto = &dtos[0];
    assert_eq!(dto.profile, "analytics");
    assert_eq!(dto.object, obj.qualified_name());
    assert_eq!(dto.kind, "table_alias");
    assert_eq!(dto.value, "catalog_items");
    assert_eq!(dto.column, None);
    assert_eq!(dto.status, ClaimStatus::Confirmed);
}
