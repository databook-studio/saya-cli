//! Scope enforcement and isolation: a scope/value mismatch is a typed error
//! both ways, two profiles hold independent values for the same kind, and
//! `unset` removes exactly the named kind at the named scope.

use super::{profile_a, profile_b, temp_root};
use saya_store::{PreferenceStore, SqliteStateStore, StoreError};
use saya_types::{DateGrain, OutputStyle, PreferenceScope, PreferenceValue};
use std::fs;

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
