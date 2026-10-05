//! M02 — the project-preferences repository: bounds, worktree isolation,
//! race-free admission, persistence, and separation from the typed
//! `user_preferences` table. The typed store keeps its own tests
//! (`tests/preferences.rs`); nothing here touches its contract.

use saya_store::{PreferenceStore, ProjectPreferenceStore, SqliteStateStore, StoreError};
use saya_types::{
    MAX_PREFERENCE_RECALL_BYTES, MAX_PREFERENCE_TEXT_BYTES, MAX_PROJECT_PREFERENCES, OutputStyle,
    PreferenceId, PreferenceScope, PreferenceValue, ProjectPreference, WorktreeId,
};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("saya-ppref-{label}-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    root
}

fn worktree_a() -> WorktreeId {
    WorktreeId::from_canonical_path("/repo/worktree-alpha").unwrap()
}

fn worktree_b() -> WorktreeId {
    WorktreeId::from_canonical_path("/repo/worktree-beta").unwrap()
}

/// A well-shaped id no store call ever minted: `pp-` + 32 hex zeros.
fn foreign_id() -> PreferenceId {
    PreferenceId::parse(&format!("pp-{}", "0".repeat(32))).unwrap()
}

async fn read_pool(db: &Path) -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(db))
        .await
        .unwrap()
}

async fn user_version(db: &Path) -> i64 {
    let pool = read_pool(db).await;
    let version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&pool)
        .await
        .unwrap();
    pool.close().await;
    version
}

async fn table_exists(db: &Path, table: &str) -> bool {
    let pool = read_pool(db).await;
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?")
            .bind(table)
            .fetch_one(&pool)
            .await
            .unwrap();
    pool.close().await;
    count > 0
}

async fn table_count(db: &Path, table: &str) -> i64 {
    let pool = read_pool(db).await;
    let sql = format!("SELECT COUNT(*) FROM {table}");
    let count: i64 = sqlx::query_scalar(&sql).fetch_one(&pool).await.unwrap();
    pool.close().await;
    count
}

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

/// Preferences survive `close()` and reopen; a forgotten row is really gone
/// from the file, not merely from the list.
#[tokio::test]
async fn preferences_persist_across_close_and_forget_removes_the_row() {
    let root = temp_root("persist");
    let db = root.join("state.sqlite3");
    let wt = worktree_a();

    let store = SqliteStateStore::new(&db);
    let stored = store
        .add_project_preference(&wt, "persistent guidance", 1_000)
        .await
        .unwrap();
    store.close().await;

    let reopened = SqliteStateStore::new(&db);
    let listed = reopened.list_project_preferences(&wt).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0], stored);
    assert_eq!(
        reopened.forget_project_preference(&wt, &stored.id).await,
        Ok(true)
    );
    reopened.close().await;
    assert_eq!(table_count(&db, "project_preferences").await, 0);

    let _ = fs::remove_dir_all(root);
}

/// The typed `PreferenceStore` is untouched: its own table holds only its own
/// rows, and a project preference's text never leaks through it.
#[tokio::test]
async fn the_typed_preference_store_is_unaffected() {
    let root = temp_root("separation");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let scope = PreferenceScope::Global;
    let wt = worktree_a();

    store
        .set_preference(&scope, PreferenceValue::output_style(OutputStyle::Compact))
        .await
        .unwrap();
    store
        .add_project_preference(&wt, "project-only guidance", 1_000)
        .await
        .unwrap();

    let typed = store
        .get_preference(&scope, "output_style")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(typed, PreferenceValue::output_style(OutputStyle::Compact));
    let typed_list = store.list_preferences(&scope).await.unwrap();
    assert_eq!(typed_list.len(), 1);
    assert_eq!(typed_list[0].0, "output_style");
    assert_eq!(table_count(&db, "user_preferences").await, 1);

    store.close().await;
    let _ = fs::remove_dir_all(root);
}

/// A fresh database reaches `user_version = 9` with the project-preferences
/// table in place.
#[tokio::test]
async fn fresh_database_reaches_version_nine_with_the_project_table() {
    let root = temp_root("fresh-v9");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    assert!(
        store
            .list_project_preferences(&worktree_a())
            .await
            .unwrap()
            .is_empty()
    );
    store.close().await;
    assert_eq!(user_version(&db).await, 9);
    assert!(
        table_exists(&db, "project_preferences").await,
        "missing project_preferences"
    );
    let _ = fs::remove_dir_all(root);
}

/// A database rewound to `user_version = 8` with the project table dropped is
/// upgraded again: the table is recreated at version 9 and other data — here
/// a typed preference — survives the ride.
#[tokio::test]
async fn rewind_to_version_eight_recreates_the_project_table() {
    let root = temp_root("rewind-v8");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store
        .set_preference(
            &PreferenceScope::Global,
            PreferenceValue::output_style(OutputStyle::Compact),
        )
        .await
        .unwrap();
    store.close().await;

    let pool = read_pool(&db).await;
    sqlx::query("DROP TABLE IF EXISTS project_preferences")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("PRAGMA user_version = 8")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    assert_eq!(user_version(&db).await, 8);

    let reopened = SqliteStateStore::new(&db);
    assert!(
        reopened
            .list_project_preferences(&worktree_a())
            .await
            .unwrap()
            .is_empty()
    );
    reopened.close().await;
    assert_eq!(user_version(&db).await, 9);
    assert!(table_exists(&db, "project_preferences").await);
    let survivor = SqliteStateStore::new(&db);
    let typed = survivor
        .get_preference(&PreferenceScope::Global, "output_style")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(typed, PreferenceValue::output_style(OutputStyle::Compact));
    survivor.close().await;

    let _ = fs::remove_dir_all(root);
}

/// Every returned record round-trips through the strict reader: version,
/// id shape, worktree shape, text bounds, and timestamps all validate.
#[tokio::test]
async fn every_listed_record_validates_as_json() {
    let root = temp_root("json");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let wt = worktree_a();

    store
        .add_project_preference(&wt, "line one\nline two", 1_000)
        .await
        .unwrap();
    let listed = store.list_project_preferences(&wt).await.unwrap();
    assert_eq!(listed.len(), 1);
    let bytes = serde_json::to_vec(&listed[0]).unwrap();
    let decoded = ProjectPreference::from_json_bytes(&bytes).unwrap();
    assert_eq!(decoded, listed[0]);

    store.close().await;
    let _ = fs::remove_dir_all(root);
}
