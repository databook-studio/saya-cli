use super::{KnowledgeItemRequest, KnowledgeItemStore, KnowledgeStoreError};
use crate::SqliteStateStore;
use saya_types::{
    ClaimOrigin, ClaimPayload, DatabaseObjectKind, DatabaseObjectRef, KnowledgeSlot,
    KnowledgeState, ProfileIdentity, SchemaFingerprint,
};
use std::{fs, time::SystemTime};

#[tokio::test]
async fn table_user_notes_round_trip_and_stop_at_four_per_table() {
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("saya-user-notes-{stamp}"));
    fs::create_dir_all(&root).unwrap();
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let profile = ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap();
    let object =
        DatabaseObjectRef::new(profile, "db", "public", "orders", DatabaseObjectKind::Table)
            .unwrap();
    let fingerprint = SchemaFingerprint::from_parts(1, &"a".repeat(64)).unwrap();

    for index in 0..4 {
        store
            .put_knowledge_item(KnowledgeItemRequest {
                object: object.clone(),
                slot: KnowledgeSlot::TableUserNote,
                value: ClaimPayload::table_user_note(format!("Note {index} verbatim.")).unwrap(),
                source: ClaimOrigin::UserExplicit,
                state: KnowledgeState::Active,
                schema_binding_json: r#"{"type":"table"}"#.into(),
                fingerprint: fingerprint.clone(),
            })
            .await
            .unwrap();
    }
    let refusal = store
        .put_knowledge_item(KnowledgeItemRequest {
            object: object.clone(),
            slot: KnowledgeSlot::TableUserNote,
            value: ClaimPayload::table_user_note("Fifth note.").unwrap(),
            source: ClaimOrigin::UserExplicit,
            state: KnowledgeState::Active,
            schema_binding_json: r#"{"type":"table"}"#.into(),
            fingerprint,
        })
        .await;
    assert!(matches!(refusal, Err(KnowledgeStoreError::BoundExceeded)));

    let items = store.knowledge_for_object(&object).await.unwrap();
    assert_eq!(items.len(), 4);
    assert!(items.iter().all(|item| {
        item.slot == KnowledgeSlot::TableUserNote
            && item.source == ClaimOrigin::UserExplicit
            && item.state == KnowledgeState::Active
            && item.schema_binding_json == r#"{"type":"table"}"#
    }));
    let mut texts: Vec<_> = items
        .iter()
        .filter_map(|item| match &item.value {
            ClaimPayload::TableUserNote { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    texts.sort_unstable();
    assert_eq!(
        texts,
        [
            "Note 0 verbatim.",
            "Note 1 verbatim.",
            "Note 2 verbatim.",
            "Note 3 verbatim."
        ]
    );
    // A binary predating `table.user_note` cannot parse the new slot. It fails
    // closed with the same typed malformed-slot error as any unknown slot.
    sqlx::query("UPDATE knowledge_items SET slot='table.future'")
        .execute(store.pool().await.unwrap())
        .await
        .unwrap();
    assert!(matches!(
        store.knowledge_for_object(&object).await,
        Err(KnowledgeStoreError::MalformedSlot)
    ));
    drop(store);
    let _ = fs::remove_dir_all(root);
}
