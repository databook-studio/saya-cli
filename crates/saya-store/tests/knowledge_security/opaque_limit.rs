use super::*;

// ---------------------------------------------------------------------------
// §4  The documented limit: opaque tokens are stored, not refused
// ---------------------------------------------------------------------------

/// A bare opaque token and a single result-row cell have no structure separating
/// them from a legitimate business term. The store admits them — and they DO
/// reach the bytes. This test is written to fail if that limit ever changes
/// silently in either direction: if the store learned to refuse this class, the
/// `is_ok` assertion fails (move the sentinel into `STRUCTURAL_SENTINELS`); if it
/// learned to scrub-and-store, the byte assertion fails (a silent partial fix is
/// the worst outcome). Keeping secrets and result cells out is the job of the
/// typed payload allow-list and the learning envelope, not of string inspection.
#[tokio::test]
async fn opaque_sentinels_are_stored_because_nothing_distinguishes_them() {
    for &sentinel in UNDETECTABLE_SENTINELS {
        let root = temp_root(&format!("opaque-{}", hash_label(sentinel)));
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

        assert!(
            result.is_ok(),
            "`{sentinel}` was refused — if the store learned to detect this class, \
             move it into STRUCTURAL_SENTINELS and delete it from here",
        );
        assert!(
            window_contains(&bytes, sentinel.as_bytes()),
            "`{sentinel}` was admitted but did not reach the bytes — a silent scrub \
             is worse than an honest admit; investigate before weakening this",
        );
    }
}
