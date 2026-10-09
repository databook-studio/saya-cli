//! The admission gate on readback: a persisted row whose JSON resembles a
//! credential or a statement is refused even though the typed constructor
//! could never have produced it.

use super::{profile_a, temp_root};
use saya_store::{PreferenceStore, SqliteStateStore, StoreError};
use saya_types::{PreferenceScope, PreferenceValue};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{fs, path::Path};

/// A preference containing a credential or SQL shape is refused by the
/// admission gate. A preference value cannot carry such shapes by
/// construction, so the gate is exercised by a hand-injected `value_json` row:
/// the store must refuse to *read back* a row whose persisted JSON looks like a
/// secret or a statement, even though the type could never have produced it.
#[tokio::test]
async fn a_credential_or_sql_shape_is_refused_by_admission_on_readback() {
    let root = temp_root("admission");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let scope = PreferenceScope::Profile(profile_a());

    // Write a legitimate value so the table and the scope_key exist.
    store
        .set_preference(&scope, PreferenceValue::timezone("Europe/London").unwrap())
        .await
        .unwrap();

    // Hand-inject a credential-shaped value_json at this scope/kind, bypassing
    // the type. A future build or direct SQL could do this; the store must not
    // surface it as a value.
    let pool = read_pool(&db).await;
    sqlx::query(
        "UPDATE user_preferences SET value_json=? WHERE scope_key=? AND preference_kind='timezone'",
    )
    .bind(r#"{"kind":"timezone","value":"postgres://user:SENTINELPASSWORD@host/db"}"#)
    .bind(scope_key(&scope))
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let err = store.get_preference(&scope, "timezone").await.unwrap_err();
    assert_eq!(err, StoreError::Invalid);
    let _ = fs::remove_dir_all(root);
}

async fn read_pool(db: &Path) -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(db))
        .await
        .unwrap()
}

/// Mirrors the store's scope-key derivation so the test can hand-inject a row at
/// the same key the store would use. Kept in sync by the admission test above.
fn scope_key(scope: &PreferenceScope) -> String {
    match scope {
        PreferenceScope::Global => "global".to_string(),
        PreferenceScope::Profile(id) => id.as_str().to_string(),
        // PreferenceScope is #[non_exhaustive]; a future variant has no store
        // representation yet, and this helper only needs the two that exist.
        _ => String::new(),
    }
}
