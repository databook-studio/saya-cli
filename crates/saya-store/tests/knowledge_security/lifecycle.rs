use super::*;

// ---------------------------------------------------------------------------
// §3  A clean full lifecycle leaves no sentinels in db or sidecars
// ---------------------------------------------------------------------------

/// Drive a full lifecycle on clean text — put, revalidate, state transition, a
/// second multi-valued item, delete — then scan db + -wal + -shm before close
/// (WAL still populated) and again after. No sentinel may appear in any file.
/// This proves the ordinary write path never smuggles one in through the back
/// door, across every write method the repository exposes.
#[tokio::test]
async fn full_lifecycle_leaves_no_sentinels_in_db_or_sidecars() {
    let root = temp_root("lifecycle");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object("orders");

    // put (insert_or_replace) — the single-valued grain slot.
    store
        .put_knowledge_item(grain_request(
            ClaimPayload::table_grain("one row per order", None).unwrap(),
            clean_binding(),
        ))
        .await
        .unwrap();
    let id = store.knowledge_for_object(&obj).await.unwrap()[0]
        .id
        .clone();

    // revalidate (revalidate_item) — a clean new binding + fingerprint version.
    let (fp2, binding2) = (
        SchemaFingerprint::from_parts(2, &"f".repeat(64)).unwrap(),
        r#"{"columns":["order_id"]}"#.to_owned(),
    );
    store
        .revalidate_knowledge_item(&id, fp2, binding2)
        .await
        .unwrap();

    // state transition (update_state) — no user text on this path, but exercise it.
    store
        .update_knowledge_item_state(&id, KnowledgeState::Dismissed)
        .await
        .unwrap();

    // a second item on a multi-valued slot (append path).
    store
        .put_knowledge_item(KnowledgeItemRequest {
            object: obj.clone(),
            slot: KnowledgeSlot::TableAlias,
            value: ClaimPayload::table_alias("orders").unwrap(),
            source: ClaimOrigin::UserExplicit,
            state: KnowledgeState::Active,
            schema_binding_json: clean_binding(),
            fingerprint: fingerprint(),
        })
        .await
        .unwrap();

    // delete (delete_item) the first item.
    store.delete_knowledge_item(&id).await.unwrap();

    // Before close: WAL still holds uncheckpointed pages.
    assert_no_sentinels("pre-close (db+wal+shm)", &db_bytes(&db));
    store.close().await;
    // After close: sidecars may have been checkpointed/removed; scan what remains.
    assert_no_sentinels("post-close (db+wal+shm)", &db_bytes(&db));
    let _ = fs::remove_dir_all(root);
}
