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

#[path = "project_preferences/bounds.rs"]
mod bounds;
#[path = "project_preferences/concurrency.rs"]
mod concurrency;
#[path = "project_preferences/input_security.rs"]
mod input_security;
#[path = "project_preferences/lifecycle.rs"]
mod lifecycle;
#[path = "project_preferences/persistence.rs"]
mod persistence;
