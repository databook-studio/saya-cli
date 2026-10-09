use saya_store::{
    AuditEntry, AuditOperation, AuditStatus, AuditStore, SchemaStore, SqliteStateStore, StoreError,
};
use saya_types::{Column, Database, MAX_SCHEMA_COLUMNS, Schema, SchemaTree, Table};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

const PROFILE: &str = "p-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn schema(table: &str) -> SchemaTree {
    SchemaTree {
        databases: vec![Database {
            name: "main".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![Table {
                    name: table.into(),
                    columns: vec![Column {
                        name: "id".into(),
                        data_type: "INTEGER".into(),
                        nullable: false,
                    }],
                    primary_key: vec![],
                    foreign_keys: vec![],
                }],
            }],
        }],
    }
}

/// A single-connection pool over the same store file, as the planted-row
/// tests use. The connection's default 5s busy timeout absorbs the write
/// lock held briefly by the sibling writer or a commit.
async fn raw_pool(db: &std::path::Path) -> sqlx::SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(db)
                .create_if_missing(true),
        )
        .await
        .unwrap()
}

/// One atomic rewrite of the cached row: bytes and metadata land together
/// in a single statement — the statement production `upsert_schema` runs
/// inside its transaction — so the row only ever holds self-consistent
/// generations.
async fn stamp_generation(
    pool: &sqlx::SqlitePool,
    profile_id: &str,
    schema_json: &str,
    updated_unix_ms: i64,
    version: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO schema_cache(profile_id, schema_json, updated_unix_ms, version) VALUES (?, ?, ?, ?) ON CONFLICT(profile_id) DO UPDATE SET schema_json=excluded.schema_json, updated_unix_ms=excluded.updated_unix_ms, version=excluded.version")
        .bind(profile_id)
        .bind(schema_json)
        .bind(updated_unix_ms)
        .bind(version)
        .execute(pool)
        .await
        .map(drop)
}
fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("saya-state-{label}-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    root
}

#[path = "state_store/audit.rs"]
mod audit;
#[path = "state_store/filesystem.rs"]
mod filesystem;
#[path = "state_store/open_and_validation.rs"]
mod open_and_validation;
#[path = "state_store/schema_consistency.rs"]
mod schema_consistency;
#[path = "state_store/schema_read_bounds.rs"]
mod schema_read_bounds;
#[path = "state_store/schema_writes.rs"]
mod schema_writes;
