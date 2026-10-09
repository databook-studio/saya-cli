use super::*;

/// Secret-shaped text is refused on add and on edit — nothing is stored,
/// and a refused edit leaves the old text intact.
#[tokio::test]
async fn secret_shaped_text_is_refused_on_add_and_edit() {
    let root = temp_root("secrets");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let wt = worktree_a();

    for secret in ["api_key=abc", "password=x", "postgres://u:p@h/db"] {
        assert_eq!(
            store.add_project_preference(&wt, secret, 1_000).await,
            Err(StoreError::Invalid),
            "secret-shaped text was stored: {secret}"
        );
    }
    assert!(
        store
            .list_project_preferences(&wt)
            .await
            .unwrap()
            .is_empty()
    );

    let honest = store
        .add_project_preference(&wt, "honest guidance", 1_000)
        .await
        .unwrap();
    assert_eq!(
        store
            .edit_project_preference(&wt, &honest.id, "token=xyz", 2_000)
            .await,
        Err(StoreError::Invalid)
    );
    let listed = store.list_project_preferences(&wt).await.unwrap();
    assert_eq!(listed[0].text, "honest guidance");

    store.close().await;
    let _ = fs::remove_dir_all(root);
}

/// Control characters other than newline and tab are refused; newline and
/// tab are allowed.
#[tokio::test]
async fn control_characters_are_refused_but_newline_and_tab_are_allowed() {
    let root = temp_root("controls");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let wt = worktree_a();

    assert_eq!(
        store
            .add_project_preference(&wt, "bell\u{7}rings", 1_000)
            .await,
        Err(StoreError::Invalid)
    );
    assert_eq!(
        store
            .add_project_preference(&wt, "carriage\rreturn", 1_000)
            .await,
        Err(StoreError::Invalid)
    );
    store
        .add_project_preference(&wt, "line one\nline two\ttabbed", 1_000)
        .await
        .unwrap();
    assert_eq!(store.list_project_preferences(&wt).await.unwrap().len(), 1);

    store.close().await;
    let _ = fs::remove_dir_all(root);
}
