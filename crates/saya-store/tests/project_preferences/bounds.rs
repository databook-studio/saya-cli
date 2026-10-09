use super::*;

/// The 64th add succeeds and the 65th is refused with the store unchanged.
#[tokio::test]
async fn the_count_bound_refuses_the_sixty_fifth_add() {
    let root = temp_root("count");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let wt = worktree_a();
    assert_eq!(MAX_PROJECT_PREFERENCES, 64);

    for i in 0..MAX_PROJECT_PREFERENCES {
        store
            .add_project_preference(&wt, &format!("guidance {i}"), 1_000)
            .await
            .unwrap();
    }
    assert_eq!(
        store
            .add_project_preference(&wt, "one too many", 1_000)
            .await,
        Err(StoreError::LimitExceeded)
    );
    assert_eq!(store.list_project_preferences(&wt).await.unwrap().len(), 64);

    store.close().await;
    let _ = fs::remove_dir_all(root);
}

/// Text bounds are exact: 1024 bytes is stored, 1025 is `Invalid`, and empty
/// or control-carrying text never reaches a row.
#[tokio::test]
async fn text_bounds_are_enforced_exactly() {
    let root = temp_root("text");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let wt = worktree_a();
    assert_eq!(MAX_PREFERENCE_TEXT_BYTES, 1024);

    let exact = "a".repeat(MAX_PREFERENCE_TEXT_BYTES);
    store
        .add_project_preference(&wt, &exact, 1_000)
        .await
        .unwrap();
    let over = "a".repeat(MAX_PREFERENCE_TEXT_BYTES + 1);
    assert_eq!(
        store.add_project_preference(&wt, &over, 1_000).await,
        Err(StoreError::Invalid)
    );
    assert_eq!(
        store.add_project_preference(&wt, "", 1_000).await,
        Err(StoreError::Invalid)
    );
    assert_eq!(
        store.add_project_preference(&wt, "   \n  ", 1_000).await,
        Err(StoreError::Invalid)
    );
    assert_eq!(store.list_project_preferences(&wt).await.unwrap().len(), 1);

    store.close().await;
    let _ = fs::remove_dir_all(root);
}

/// The aggregate recall bound: sixteen 1024-byte texts fill the 16 KiB cap
/// exactly; the seventeenth add is refused with nothing written.
#[tokio::test]
async fn the_aggregate_bound_refuses_adds_past_sixteen_kib() {
    let root = temp_root("aggregate");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let wt = worktree_a();
    assert_eq!(MAX_PREFERENCE_RECALL_BYTES, 16 * 1024);

    let full = "a".repeat(MAX_PREFERENCE_TEXT_BYTES);
    for _ in 0..16 {
        store
            .add_project_preference(&wt, &full, 1_000)
            .await
            .unwrap();
    }
    assert_eq!(
        store.add_project_preference(&wt, "one over", 1_000).await,
        Err(StoreError::LimitExceeded)
    );
    assert_eq!(store.list_project_preferences(&wt).await.unwrap().len(), 16);
    assert_eq!(table_count(&db, "project_preferences").await, 16);

    store.close().await;
    let _ = fs::remove_dir_all(root);
}

/// An edit that would push its worktree past the aggregate cap is refused
/// and the old text stays intact. Seventeen 900-byte texts leave headroom;
/// after eight edits to 1024 bytes the ninth would exceed 16 KiB.
#[tokio::test]
async fn an_edit_that_would_exceed_the_aggregate_is_refused_with_the_old_text_intact() {
    let root = temp_root("edit-aggregate");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let wt = worktree_a();

    let base = "n".repeat(900);
    let mut ids = Vec::new();
    for _ in 0..17 {
        let record = store
            .add_project_preference(&wt, &base, 1_000)
            .await
            .unwrap();
        ids.push(record.id);
    }
    let grown = "g".repeat(MAX_PREFERENCE_TEXT_BYTES);
    for id in &ids[..8] {
        store
            .edit_project_preference(&wt, id, &grown, 2_000)
            .await
            .unwrap();
    }
    assert_eq!(
        store
            .edit_project_preference(&wt, &ids[8], &grown, 2_000)
            .await,
        Err(StoreError::LimitExceeded)
    );
    let listed = store.list_project_preferences(&wt).await.unwrap();
    let ninth = listed.iter().find(|p| p.id == ids[8]).unwrap();
    assert_eq!(ninth.text, base);

    store.close().await;
    let _ = fs::remove_dir_all(root);
}
