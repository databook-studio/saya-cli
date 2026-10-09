use super::*;

/// A041: a persisted `fingerprint_version` outside `u32` fails closed. The
/// write path can only persist a `u32` version (it binds `version as i64`), so
/// negative and over-`u32` rows arrive only from an incompatible writer — the
/// fixtures persist them directly and assert the read rejects them with the
/// typed `Store(Invalid)` error, on both the single-row and the paged path.
#[tokio::test]
async fn persisted_out_of_range_fingerprint_versions_are_rejected_as_invalid() {
    for (label, version) in [("negative", -1_i64), ("over-u32", i64::from(u32::MAX) + 1)] {
        let root = temp_root(&format!("fingerprint-{label}"));
        let db = root.join("state.sqlite3");
        let store = SqliteStateStore::new(&db);
        let obj = object(&profile('a'), "orders");
        store
            .put_knowledge_item(request(
                &obj,
                KnowledgeSlot::TableGrain,
                ClaimPayload::table_grain("one row per order", None).unwrap(),
                ClaimOrigin::UserExplicit,
                KnowledgeState::Active,
                1,
            ))
            .await
            .unwrap();
        let id = store.knowledge_for_object(&obj).await.unwrap().remove(0).id;

        let pool = read_pool(&db).await;
        sqlx::query("UPDATE knowledge_items SET fingerprint_version=? WHERE id=?")
            .bind(version)
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;

        assert_eq!(
            store.get_knowledge_item(&id).await,
            Err(KnowledgeStoreError::Store(StoreError::Invalid)),
            "{label} fingerprint_version ({version}) must fail closed as Invalid"
        );
        assert_eq!(
            store.knowledge_for_object(&obj).await,
            Err(KnowledgeStoreError::Store(StoreError::Invalid)),
            "{label} fingerprint_version ({version}) must fail closed on the paged path too"
        );
        let _ = fs::remove_dir_all(root);
    }
}
