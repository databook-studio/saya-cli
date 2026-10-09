use super::*;

// ---------------------------------------------------------------------------
// §5  A directive claim's `reason` is a persisted free-text channel too
// ---------------------------------------------------------------------------
//
// The reason (spec: claim-reasons) added a *third* place user-supplied text
// reaches disk, alongside the value and the schema binding. Every test above
// passes `None` for it, so without these two the reason would be the one
// persisted channel no byte-scan covers — and this suite exists because a
// guarantee that is only asserted through the API is not asserted at all.

/// Structural secrets must be refused in the reason exactly as in the value.
/// `validate_reason` routes through the same `validate_text` the description
/// uses, so this asserts that routing holds rather than trusting that it does.
#[tokio::test]
async fn structural_secret_in_the_reason_is_refused_and_never_reaches_the_bytes() {
    for &sentinel in STRUCTURAL_SENTINELS {
        let root = temp_root(&format!("reason-{}", hash_label(sentinel)));
        let db = root.join("state.sqlite3");
        let store = SqliteStateStore::new(&db);

        // The value is clean; only the reason carries the sentinel, so a pass
        // here cannot come from the value's admission check.
        let payload = ClaimPayload::table_grain("one row per order", Some(sentinel));
        let result = match payload {
            // Refused at construction is the better outcome: it never reaches
            // the store at all. Either way the bytes must stay clean.
            Err(_) => Err(KnowledgeStoreError::Store(StoreError::Invalid)),
            Ok(value) => {
                store
                    .put_knowledge_item(grain_request(value, clean_binding()))
                    .await
            }
        };
        let bytes = db_bytes(&db);
        store.close().await;
        let _ = fs::remove_dir_all(root);

        assert_eq!(
            result,
            Err(KnowledgeStoreError::Store(StoreError::Invalid)),
            "`{sentinel}` was admitted as a reason; the admission gate must refuse it",
        );
        assert!(
            !window_contains(&bytes, sentinel.as_bytes()),
            "LEAK: `{sentinel}` reached the database bytes via the reason",
        );
    }
}

/// `forget` must erase the reason from the bytes, not only the value. The
/// payload's `blanked()` clears both, and this proves that on disk: a user who
/// forgets a claim is entitled to have the sentence explaining it gone too,
/// and a reason is frequently the more revealing half of the pair.
#[tokio::test]
async fn forget_erases_the_reason_from_the_bytes_not_only_the_value() {
    const REASON_SENTINEL: &str = "SENTINELFORGETREASON";

    let root = temp_root("forget-erases-reason");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object("orders");

    // Ordinary business text in both channels, so admission takes it and both
    // reach the bytes. The value and the reason are deliberately different
    // sentinels: a single one could pass by erasing only one channel.
    let value = ClaimPayload::table_grain(FORGET_SENTINEL, Some(REASON_SENTINEL)).unwrap();
    store
        .put_knowledge_item(grain_request(value, clean_binding()))
        .await
        .unwrap();
    let id = store.knowledge_for_object(&obj).await.unwrap()[0]
        .id
        .clone();

    // The red half: both must really be on disk, or the erasure assertion
    // below would pass against a reason that was never stored.
    assert!(
        window_contains(&db_bytes(&db), REASON_SENTINEL.as_bytes()),
        "precondition: the reason must reach the bytes on the write, or the \
         post-forget assertion proves nothing"
    );

    store.forget_knowledge_item(&id).await.unwrap();

    assert!(
        !window_contains(&db_bytes(&db), REASON_SENTINEL.as_bytes()),
        "LEAK: the forgotten reason survived in the database bytes — forget \
         must erase the reason as well as the value"
    );
    assert!(
        !window_contains(&db_bytes(&db), FORGET_SENTINEL.as_bytes()),
        "LEAK: the forgotten value survived alongside the reason"
    );
    store.close().await;
    assert!(
        !window_contains(&db_bytes(&db), REASON_SENTINEL.as_bytes()),
        "LEAK: the forgotten reason survived in the database bytes after close"
    );

    let _ = fs::remove_dir_all(root);
}
