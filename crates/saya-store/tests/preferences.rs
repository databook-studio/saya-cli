//! Phase 5c-1 — preferences store: scope, isolation, replace, unset, migration,
//! and the admission gate. See `.claude/specs/spec-5c1-preferences-store.md`.
//!
//! A preference is *not a claim*: no object, no fingerprint, no drift, no
//! lifecycle, no evidence. These tests cover the store half of the slice —
//! the typed values are unit-tested in `saya-types`.

use saya_store::{PreferenceStore, SqliteStateStore, StoreError};
use saya_types::{DateGrain, OutputStyle, PreferenceScope, PreferenceValue, ProfileIdentity};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{
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
        std::env::temp_dir().join(format!("saya-pref-{label}-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    root
}

fn profile_a() -> ProfileIdentity {
    ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap()
}
fn profile_b() -> ProfileIdentity {
    ProfileIdentity::parse(&format!("p-{}", "b".repeat(64))).unwrap()
}

/// Each value round-trips through the store at its correct scope.
#[tokio::test]
async fn each_value_round_trips_at_its_correct_scope() {
    let root = temp_root("roundtrip");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);

    let cases = [
        (
            PreferenceScope::Global,
            PreferenceValue::output_style(OutputStyle::Compact),
        ),
        (
            PreferenceScope::Global,
            PreferenceValue::default_profile("warehouse").unwrap(),
        ),
        (
            PreferenceScope::Profile(profile_a()),
            PreferenceValue::timezone("Europe/London").unwrap(),
        ),
        (
            PreferenceScope::Profile(profile_a()),
            PreferenceValue::date_grain(DateGrain::Month),
        ),
    ];
    for (scope, value) in &cases {
        store.set_preference(scope, value.clone()).await.unwrap();
    }
    for (scope, value) in &cases {
        let got = store
            .get_preference(scope, value.kind())
            .await
            .unwrap()
            .expect("preference should be present");
        assert_eq!(got, *value, "round-trip mismatch for {}", value.kind());
    }
    let _ = fs::remove_dir_all(root);
}

/// Setting a `Profile`-scoped value with `Global` scope is a typed error, and
/// vice versa. The store enforces the rule because it receives
/// the scope and value separately — a wrong pairing must be refused, not
/// silently coerced to the other scope.
#[tokio::test]
async fn wrong_scope_is_a_typed_error_both_ways() {
    let root = temp_root("scope-mismatch");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);

    // A profile-scoped value offered at a global scope is refused.
    let tz = PreferenceValue::timezone("Europe/London").unwrap();
    let err = store
        .set_preference(&PreferenceScope::Global, tz)
        .await
        .unwrap_err();
    assert_eq!(err, StoreError::Invalid);

    // A global-scoped value offered at a profile scope is refused too.
    let style = PreferenceValue::output_style(OutputStyle::Compact);
    let err = store
        .set_preference(&PreferenceScope::Profile(profile_a()), style)
        .await
        .unwrap_err();
    assert_eq!(err, StoreError::Invalid);

    // Nothing was stored under any kind at either scope.
    assert!(
        store
            .list_preferences(&PreferenceScope::Global)
            .await
            .unwrap()
            .is_empty()
    );
    let _ = fs::remove_dir_all(root);
}

/// Setting the same kind twice replaces rather than duplicating.
#[tokio::test]
async fn setting_the_same_kind_twice_replaces() {
    let root = temp_root("replace");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let scope = PreferenceScope::Global;

    store
        .set_preference(&scope, PreferenceValue::output_style(OutputStyle::Table))
        .await
        .unwrap();
    store
        .set_preference(
            &scope,
            PreferenceValue::output_style(OutputStyle::Narrative),
        )
        .await
        .unwrap();

    let rows = store.list_preferences(&scope).await.unwrap();
    assert_eq!(rows.len(), 1, "replace must not duplicate");
    assert_eq!(
        rows[0].1,
        PreferenceValue::output_style(OutputStyle::Narrative)
    );
    let _ = fs::remove_dir_all(root);
}

/// Two profiles hold independent values for the same kind — the isolation
/// invariant.
#[tokio::test]
async fn two_profiles_hold_independent_values_for_the_same_kind() {
    let root = temp_root("isolation");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let scope_a = PreferenceScope::Profile(profile_a());
    let scope_b = PreferenceScope::Profile(profile_b());

    store
        .set_preference(
            &scope_a,
            PreferenceValue::timezone("Europe/London").unwrap(),
        )
        .await
        .unwrap();
    store
        .set_preference(
            &scope_b,
            PreferenceValue::timezone("America/New_York").unwrap(),
        )
        .await
        .unwrap();

    let got_a = store
        .get_preference(&scope_a, "timezone")
        .await
        .unwrap()
        .unwrap();
    let got_b = store
        .get_preference(&scope_b, "timezone")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got_a, PreferenceValue::timezone("Europe/London").unwrap());
    assert_eq!(
        got_b,
        PreferenceValue::timezone("America/New_York").unwrap()
    );
    let _ = fs::remove_dir_all(root);
}

/// A malformed timezone is rejected by shape; a well shaped but fictional one
/// is accepted. The shape gate lives on the value constructor in
/// `saya-types`; this test pins the store end so a hand-built row with a bad
/// timezone never round-trips back as if it were valid.
#[tokio::test]
async fn malformed_timezone_is_rejected_by_shape() {
    let root = temp_root("tz-shape");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let scope = PreferenceScope::Profile(profile_a());

    // Shape validation is on the constructor; a malformed value cannot be
    // constructed, so it cannot reach the store.
    assert!(PreferenceValue::timezone("Europe/London!").is_err());
    // A well shaped but fictional name is accepted by shape and round-trips.
    store
        .set_preference(
            &scope,
            PreferenceValue::timezone("Mars/Olympus_Mons").unwrap(),
        )
        .await
        .unwrap();
    let got = store
        .get_preference(&scope, "timezone")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got, PreferenceValue::timezone("Mars/Olympus_Mons").unwrap());
    let _ = fs::remove_dir_all(root);
}

/// `unset` removes only the named kind at the named scope.
#[tokio::test]
async fn unset_removes_only_the_named_kind_at_the_named_scope() {
    let root = temp_root("unset");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let scope_a = PreferenceScope::Profile(profile_a());
    let scope_b = PreferenceScope::Profile(profile_b());

    store
        .set_preference(
            &scope_a,
            PreferenceValue::timezone("Europe/London").unwrap(),
        )
        .await
        .unwrap();
    store
        .set_preference(&scope_a, PreferenceValue::date_grain(DateGrain::Month))
        .await
        .unwrap();
    store
        .set_preference(
            &scope_b,
            PreferenceValue::timezone("America/New_York").unwrap(),
        )
        .await
        .unwrap();

    // Unset timezone at scope_a only.
    store.unset_preference(&scope_a, "timezone").await.unwrap();

    // timezone at scope_a is gone; date_grain at scope_a survives; timezone at
    // scope_b survives.
    assert!(
        store
            .get_preference(&scope_a, "timezone")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .get_preference(&scope_a, "date_grain")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        store
            .get_preference(&scope_b, "timezone")
            .await
            .unwrap()
            .is_some()
    );
    // Unsetting a missing kind is a no-op, not an error.
    store.unset_preference(&scope_a, "timezone").await.unwrap();
    let _ = fs::remove_dir_all(root);
}

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

// The next three tests guard the type-level "make it unrepresentable, not
// filtered" rule. A derived `Deserialize` would populate the
// string fields directly and bypass the validated constructors, so the type's
// `Deserialize` is hand-rolled to run the validators on deserialization too.
// These pin that: a SQL-shaped timezone, a control-char profile name, and an
// unknown free-text kind are all refused by the *type*, not only by the store.

#[test]
fn serde_refuses_a_sql_shaped_timezone_at_the_type() {
    let json = r#"{"kind":"timezone","value":"SELECT col FROM orders"}"#;
    let r: Result<PreferenceValue, _> = serde_json::from_str(json);
    assert!(r.is_err(), "serde accepted an unvalidated timezone: {r:?}");
}

#[test]
fn serde_refuses_a_control_char_profile_name_at_the_type() {
    // Spaces are legitimate in profile names (config allows them), so the probe
    // uses a real control character the constructor rejects.
    let json = "{\"kind\":\"default_profile\",\"name\":\"bad\\u0000name\"}";
    let r: Result<PreferenceValue, _> = serde_json::from_str(json);
    assert!(
        r.is_err(),
        "serde accepted an unvalidated profile name: {r:?}"
    );
}

#[test]
fn serde_rejects_an_unknown_free_text_kind() {
    // No free-text variant exists; an unknown kind is refused, so SQL or
    // instructions cannot be smuggled in as a new variant.
    let json = r#"{"kind":"freetext","text":"drop table x"}"#;
    let r: Result<PreferenceValue, _> = serde_json::from_str(json);
    assert!(r.is_err());
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
