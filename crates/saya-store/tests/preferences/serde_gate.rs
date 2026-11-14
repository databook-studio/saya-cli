//! The next three tests guard the type-level "make it unrepresentable, not
//! filtered" rule. A derived `Deserialize` would populate the
//! string fields directly and bypass the validated constructors, so the type's
//! `Deserialize` is hand-rolled to run the validators on deserialization too.
//! These pin that: a SQL-shaped timezone, a control-char profile name, and an
//! unknown free-text kind are all refused by the *type*, not only by the store.

use saya_types::PreferenceValue;

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
