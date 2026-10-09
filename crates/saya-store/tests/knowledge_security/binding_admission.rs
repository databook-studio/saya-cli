use super::*;

// ---------------------------------------------------------------------------
// §2  Structural secrets in the schema binding are refused too
// ---------------------------------------------------------------------------
//
// The binding is a second persisted channel the legacy suite did not have. The
// production gate checks it with the same `redact` + `admission::check` pair
// (`writes.rs:44-47`), so a sentinel smuggled in through `schema_binding_json`
// must be refused for the same reason — and must not reach the bytes.

/// For each structural sentinel, store a clean value but carry the sentinel in
/// `schema_binding_json`. The binding gate must refuse it and store nothing.
#[tokio::test]
async fn structural_secret_in_the_schema_binding_is_refused_and_never_reaches_the_bytes() {
    for &sentinel in STRUCTURAL_SENTINELS {
        let root = temp_root(&format!("bind-{}", hash_label(sentinel)));
        let db = root.join("state.sqlite3");
        let store = SqliteStateStore::new(&db);

        let result = store
            .put_knowledge_item(grain_request(
                ClaimPayload::table_grain("one row per order", None).unwrap(),
                sentinel.to_owned(),
            ))
            .await;
        let bytes = db_bytes(&db);
        store.close().await;
        let _ = fs::remove_dir_all(root);

        assert_eq!(
            result,
            Err(KnowledgeStoreError::Store(StoreError::Invalid)),
            "`{sentinel}` in the schema binding was admitted; the binding gate must refuse it",
        );
        assert!(
            !window_contains(&bytes, sentinel.as_bytes()),
            "LEAK: `{sentinel}` reached the database bytes via the schema binding",
        );
    }
}
