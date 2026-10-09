use super::*;

// ---------------------------------------------------------------------------
// §5  forget erases a fact's content from the bytes, not just from the API
// ---------------------------------------------------------------------------
//
// The deletion promise (`docs/memory.md` §Deletion) is that `forget` clears a
// fact's payload and referenced columns while keeping the row. The regression
// the migration introduced: `forget` flipped only the state, so the payload
// stayed in `value_json`. This test restores the guarantee the legacy store
// carried — verified at the byte level, because a value cleared in Rust but
// still resident in a SQLite page is not erased. It scans the database file
// and its `-wal`/`-shm` sidecars (WAL mode keeps freshly-written pages in
// `-wal` until checkpoint), so a sentinel that survived in any of them fails.

/// `forget` erases a fact's value from the database bytes while keeping the row,
/// and a re-remember of the same fact reports *previously forgotten* rather than
/// silently resurrecting it. Planted sentinel: a free-text grain the admission
/// gate admits, so it reaches the bytes on the write; after `forget`, it must
/// be absent from the database file *and* the `-wal`/`-shm` sidecars — not merely
/// absent from a getter's return, which would say nothing about a stale page.
#[tokio::test]
async fn forget_erases_the_value_from_the_bytes_and_keeps_the_row() {
    let root = temp_root("forget-erases");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object("orders");

    // Write a grain whose description carries the sentinel. Admission admits
    // ordinary business text, so the sentinel reaches the bytes here.
    let value = ClaimPayload::table_grain(FORGET_SENTINEL, None).unwrap();
    store
        .put_knowledge_item(grain_request(value, clean_binding()))
        .await
        .unwrap();
    let id = store.knowledge_for_object(&obj).await.unwrap()[0]
        .id
        .clone();
    let created = store
        .get_knowledge_item(&id)
        .await
        .unwrap()
        .unwrap()
        .created_unix_ms;

    // Before forget: the sentinel is in the bytes (the write put it there).
    // This is the "red" half of the assertion — proving the sentinel was
    // really stored, not that the test never planted it.
    assert!(
        window_contains(&db_bytes(&db), FORGET_SENTINEL.as_bytes()),
        "precondition: the sentinel must reach the bytes on the write, or the \
         post-forget assertion would prove nothing"
    );

    store.forget_knowledge_item(&id).await.unwrap();

    // After forget: the sentinel is gone from the database file and every
    // sidecar. A getter returning None is not enough — a cleared value still
    // resident in a page is not erased, and this scan is what makes the check
    // honest. Scan before close (WAL may hold the only copy) and after.
    assert!(
        !window_contains(&db_bytes(&db), FORGET_SENTINEL.as_bytes()),
        "LEAK: the forgotten sentinel survived in the database bytes — forget \
         must erase the value, not just flip the state"
    );
    store.close().await;
    assert!(
        !window_contains(&db_bytes(&db), FORGET_SENTINEL.as_bytes()),
        "LEAK: the forgotten sentinel survived in the database bytes after close"
    );

    // The row itself remains: same id, same object, same slot, `Dismissed`, and
    // the original `created` timestamp — "why did SAYA stop using that?" stays
    // answerable. A separate store re-open proves it persisted this way.
    let store = SqliteStateStore::new(&db);
    let item = store
        .get_knowledge_item(&id)
        .await
        .unwrap()
        .expect("the forgotten row remains");
    assert_eq!(item.id, id);
    assert_eq!(item.object, obj);
    assert_eq!(item.slot, KnowledgeSlot::TableGrain);
    assert_eq!(item.state, KnowledgeState::Dismissed);
    assert_eq!(
        item.created_unix_ms, created,
        "a forgotten row keeps its created timestamp"
    );
    // The value is blanked: the grain's free-text description is empty, not the
    // sentinel. (The variant stays `TableGrain` so the row still decodes.)
    assert!(
        matches!(item.value, ClaimPayload::TableGrain { ref description, .. } if description.is_empty()),
        "the forgotten row's value is blanked, not the original text: {:?}",
        item.value
    );
    // The referenced-column binding is cleared: a `Table` binding names no
    // column, where the written row carried `{"columns":["user_id"]}`.
    assert_eq!(
        item.schema_binding_json, r#"{"type":"table"}"#,
        "the forgotten row's schema binding is cleared of referenced columns"
    );

    // Re-remembering the same fact lands on the same row (its id is
    // value-independent for the single-valued grain slot) and reports
    // *previously forgotten* — it must not silently resurrect. The tombstone
    // stays dismissed; no second row appears.
    let again = grain_request(
        ClaimPayload::table_grain(FORGET_SENTINEL, None).unwrap(),
        clean_binding(),
    );
    // `put_knowledge_item` is the write `remember` uses; for a single-valued
    // slot the ON CONFLICT(id) would overwrite the tombstone and revive it, so
    // the `remember` operation looks the row up first and does not write. This
    // test cannot call the `remember` op (it lives in saya-cli), so it asserts
    // the precondition the op relies on: the tombstone is still at the id a
    // re-remember would look up, still dismissed, with the value blanked.
    let _ = again;
    assert_eq!(
        store.knowledge_for_object(&obj).await.unwrap().len(),
        1,
        "the forgotten tombstone is the one row for the object"
    );
    assert_eq!(
        store.get_knowledge_item(&id).await.unwrap().unwrap().state,
        KnowledgeState::Dismissed,
        "the tombstone stayed dismissed"
    );

    store.close().await;
    let _ = fs::remove_dir_all(root);
}

/// `forget` of an unknown id is a typed `NotFound`, not a silent no-op — the
/// caller asked to forget a fact that is not there, and a silent success would
/// let a caller believe a fact was forgotten when none was.
#[tokio::test]
async fn forget_of_an_unknown_id_is_not_found_not_a_silent_noop() {
    let root = temp_root("forget-unknown");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);

    let result = store.forget_knowledge_item("ki-deadbeef").await;
    store.close().await;
    let _ = fs::remove_dir_all(root);

    assert_eq!(
        result,
        Err(KnowledgeStoreError::Store(StoreError::NotFound)),
        "forgetting a fact that is not there is a typed NotFound, not a silent ok"
    );
}

/// A committed tombstone must not be reported as an ordinary failure when the
/// post-commit byte cleanup is unavailable. Retrying the same id completes the
/// physical cleanup and never attempts to resurrect the logical deletion.
#[tokio::test]
async fn forget_reports_pending_cleanup_and_recovers_on_retry() {
    let root = temp_root("forget-cleanup-retry");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object("orders");
    store
        .put_knowledge_item(grain_request(
            ClaimPayload::table_grain(FORGET_SENTINEL, None).unwrap(),
            clean_binding(),
        ))
        .await
        .unwrap();
    let id = store.knowledge_for_object(&obj).await.unwrap()[0]
        .id
        .clone();

    store.fail_next_cleanup_for_tests();
    assert_eq!(
        store.forget_knowledge_item(&id).await.unwrap(),
        ForgetOutcome::CleanupPending
    );
    let item = store.get_knowledge_item(&id).await.unwrap().unwrap();
    assert_eq!(item.state, KnowledgeState::Dismissed);
    assert_eq!(item.cleanup, CleanupState::Pending);

    assert_eq!(
        store.forget_knowledge_item(&id).await.unwrap(),
        ForgetOutcome::Cleaned
    );
    let item = store.get_knowledge_item(&id).await.unwrap().unwrap();
    assert_eq!(item.cleanup, CleanupState::Complete);
    assert!(!window_contains(&db_bytes(&db), FORGET_SENTINEL.as_bytes()));
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn reopening_recovers_pending_cleanup_before_reads() {
    let root = temp_root("forget-cleanup-reopen");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object("orders");
    store
        .put_knowledge_item(grain_request(
            ClaimPayload::table_grain(FORGET_SENTINEL, None).unwrap(),
            clean_binding(),
        ))
        .await
        .unwrap();
    let id = store.knowledge_for_object(&obj).await.unwrap()[0]
        .id
        .clone();
    store.fail_next_cleanup_for_tests();
    assert_eq!(
        store.forget_knowledge_item(&id).await.unwrap(),
        ForgetOutcome::CleanupPending
    );
    store.close().await;

    let reopened = SqliteStateStore::new(&db);
    let item = reopened.get_knowledge_item(&id).await.unwrap().unwrap();
    assert_eq!(item.cleanup, CleanupState::Complete);
    assert!(!window_contains(&db_bytes(&db), FORGET_SENTINEL.as_bytes()));
    reopened.close().await;
    let _ = fs::remove_dir_all(root);
}
