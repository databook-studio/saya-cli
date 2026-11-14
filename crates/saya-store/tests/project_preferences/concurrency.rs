use super::*;

/// Eighty concurrent adds on one worktree: exactly 64 succeed and 16 are
/// refused — the count bound is enforced race-free, not count-then-race.
#[tokio::test]
async fn eighty_concurrent_adds_respect_the_count_bound() {
    let root = temp_root("race");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let wt = worktree_a();

    let mut handles = Vec::new();
    for i in 0..80_u32 {
        let store = store.clone();
        let wt = wt.clone();
        handles.push(tokio::spawn(async move {
            store
                .add_project_preference(&wt, &format!("pref-{i}"), 1_000)
                .await
        }));
    }
    let (mut ok, mut limited) = (0, 0);
    for handle in handles {
        match handle.await.unwrap() {
            Ok(_) => ok += 1,
            Err(StoreError::LimitExceeded) => limited += 1,
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }
    assert_eq!(ok, 64, "exactly the bound may store");
    assert_eq!(limited, 16, "every racer past the bound is refused");
    assert_eq!(store.list_project_preferences(&wt).await.unwrap().len(), 64);

    store.close().await;
    let _ = fs::remove_dir_all(root);
}
