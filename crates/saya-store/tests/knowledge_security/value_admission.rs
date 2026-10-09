use super::*;

// ---------------------------------------------------------------------------
// §1  Structural secrets in the value are refused and never reach the bytes
// ---------------------------------------------------------------------------

/// For each structural sentinel, attempt to store it as a `table.grain` value.
/// Every one must reject with `Store(Invalid)` and leave clean bytes — the
/// admission check runs before any INSERT, so a refused write stores nothing.
#[tokio::test]
async fn structural_secret_in_the_value_is_refused_and_never_reaches_the_bytes() {
    for &sentinel in STRUCTURAL_SENTINELS {
        let root = temp_root(&format!("val-{}", hash_label(sentinel)));
        let db = root.join("state.sqlite3");
        let store = SqliteStateStore::new(&db);

        let result = store
            .put_knowledge_item(grain_request(
                ClaimPayload::table_grain(sentinel, None).unwrap(),
                clean_binding(),
            ))
            .await;
        let bytes = db_bytes(&db);
        store.close().await;
        let _ = fs::remove_dir_all(root);

        assert_eq!(
            result,
            Err(KnowledgeStoreError::Store(StoreError::Invalid)),
            "`{sentinel}` was admitted; the admission gate must refuse it",
        );
        assert!(
            !window_contains(&bytes, sentinel.as_bytes()),
            "LEAK: `{sentinel}` reached the database bytes via the value",
        );
    }
}
