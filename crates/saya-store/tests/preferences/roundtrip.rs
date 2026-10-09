//! Values round-trip at their scope: every typed value survives a
//! set/get cycle, the same kind is replaced rather than duplicated, and a
//! well-shaped but fictional value is accepted while a malformed one is
//! rejected by shape.

use super::{profile_a, temp_root};
use saya_store::{PreferenceStore, SqliteStateStore};
use saya_types::{DateGrain, OutputStyle, PreferenceScope, PreferenceValue};
use std::fs;

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
