use super::*;

/// Add / list / edit / forget round trip on one worktree: creation order is
/// stable (same stamp falls back to id order), an edit keeps id and created
/// stamp while moving updated forward, and every remove is exact.
#[tokio::test]
async fn add_list_edit_forget_round_trips_in_creation_order() {
    let root = temp_root("roundtrip");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let wt = worktree_a();

    let alpha = store
        .add_project_preference(&wt, "alpha guidance", 1_000)
        .await
        .unwrap();
    let beta = store
        .add_project_preference(&wt, "beta guidance", 2_000)
        .await
        .unwrap();
    let gamma = store
        .add_project_preference(&wt, "gamma guidance", 3_000)
        .await
        .unwrap();

    let listed = store.list_project_preferences(&wt).await.unwrap();
    assert_eq!(
        listed.iter().map(|p| p.id.clone()).collect::<Vec<_>>(),
        vec![alpha.id.clone(), beta.id.clone(), gamma.id.clone()]
    );
    assert_eq!(listed[0].text, "alpha guidance");

    let edited = store
        .edit_project_preference(&wt, &beta.id, "beta revised", 4_000)
        .await
        .unwrap();
    assert_eq!(edited.id, beta.id);
    assert_eq!(edited.created_unix_ms, beta.created_unix_ms);
    assert_eq!(edited.updated_unix_ms, 4_000);
    assert_eq!(edited.text, "beta revised");

    let listed = store.list_project_preferences(&wt).await.unwrap();
    assert_eq!(listed[1].text, "beta revised");
    assert_eq!(listed[1].id, beta.id);

    assert_eq!(
        store.forget_project_preference(&wt, &beta.id).await,
        Ok(true)
    );
    let listed = store.list_project_preferences(&wt).await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(
        store.forget_project_preference(&wt, &beta.id).await,
        Ok(false)
    );
    assert_eq!(
        store.forget_project_preference(&wt, &foreign_id()).await,
        Ok(false)
    );

    assert_eq!(store.forget_all_project_preferences(&wt).await, Ok(2));
    assert!(
        store
            .list_project_preferences(&wt)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(store.forget_all_project_preferences(&wt).await, Ok(0));

    store.close().await;
    let _ = fs::remove_dir_all(root);
}

/// Ids minted across many adds are unique and well-shaped. One worktree
/// holds at most 64 rows, so the hundred adds span two worktrees — the ids
/// minted across the whole run are still all distinct.
#[tokio::test]
async fn ids_are_unique_across_one_hundred_adds() {
    let root = temp_root("ids");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let (wt_a, wt_b) = (worktree_a(), worktree_b());

    let mut ids = HashSet::new();
    for i in 0..100 {
        let worktree = if i < 64 { wt_a.clone() } else { wt_b.clone() };
        let record = store
            .add_project_preference(&worktree, &format!("guidance {i}"), 1_000)
            .await
            .unwrap();
        assert!(record.id.as_str().starts_with("pp-"));
        assert_eq!(record.id.as_str().len(), 35);
        ids.insert(record.id);
    }
    assert_eq!(ids.len(), 100);

    store.close().await;
    let _ = fs::remove_dir_all(root);
}

/// Rows of one worktree are invisible to another: list, edit, and forget
/// through B never see or touch A's rows, and `forget_all` is per worktree.
#[tokio::test]
async fn rows_from_one_worktree_are_invisible_to_another() {
    let root = temp_root("isolation");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let wt_a = worktree_a();
    let wt_b = worktree_b();

    let alpha = store
        .add_project_preference(&wt_a, "alpha guidance", 1_000)
        .await
        .unwrap();
    let beta = store
        .add_project_preference(&wt_b, "beta guidance", 2_000)
        .await
        .unwrap();

    let listed_b = store.list_project_preferences(&wt_b).await.unwrap();
    assert_eq!(listed_b.len(), 1);
    assert_eq!(listed_b[0].id, beta.id);

    // Through B, A's id is absent — editing is NotFound, forgetting is a
    // no-op `false`.
    assert_eq!(
        store
            .edit_project_preference(&wt_b, &alpha.id, "tampered", 5_000)
            .await,
        Err(StoreError::NotFound)
    );
    assert_eq!(
        store.forget_project_preference(&wt_b, &alpha.id).await,
        Ok(false)
    );
    assert_eq!(store.forget_all_project_preferences(&wt_b).await, Ok(1));

    // B's sweep did not touch A's row.
    let listed_a = store.list_project_preferences(&wt_a).await.unwrap();
    assert_eq!(listed_a.len(), 1);
    assert_eq!(listed_a[0].id, alpha.id);
    assert_eq!(listed_a[0].text, "alpha guidance");

    assert_eq!(store.forget_all_project_preferences(&wt_a).await, Ok(1));
    assert!(
        store
            .list_project_preferences(&wt_a)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .list_project_preferences(&wt_b)
            .await
            .unwrap()
            .is_empty()
    );

    store.close().await;
    let _ = fs::remove_dir_all(root);
}
