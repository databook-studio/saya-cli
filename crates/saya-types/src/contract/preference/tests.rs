use super::*;

#[test]
fn timezone_accepts_well_shaped_real_name() {
    let v = PreferenceValue::timezone("Europe/London").unwrap();
    assert_eq!(v.kind(), "timezone");
    assert_eq!(v.timezone_value(), Some("Europe/London"));
    assert_eq!(v.required_scope(), ScopeRequirement::Profile);
}

#[test]
fn timezone_accepts_well_shaped_fictional_name() {
    // Shape only: a fictional-but-well-shaped name is a user error they can
    // see and fix, not a rejection at storage. Bundling an IANA list is out.
    assert!(PreferenceValue::timezone("Mars/Olympus_Mons").is_ok());
    assert!(PreferenceValue::timezone("Etc/GMT+5").is_ok());
}

#[test]
fn timezone_rejects_malformed_shapes() {
    assert!(PreferenceValue::timezone("").is_err());
    assert!(PreferenceValue::timezone("Europe/London!").is_err());
    assert!(PreferenceValue::timezone("has space").is_err());
    assert!(PreferenceValue::timezone("Europe\\London").is_err());
    assert!(PreferenceValue::timezone("Europe\nLondon").is_err());
    assert!(PreferenceValue::timezone("x".repeat(MAX_TIMEZONE_CHARS + 1)).is_err());
}

#[test]
fn date_grain_round_trips_and_is_profile_scoped() {
    for grain in [
        DateGrain::Day,
        DateGrain::Week,
        DateGrain::Month,
        DateGrain::Quarter,
        DateGrain::Year,
    ] {
        let v = PreferenceValue::date_grain(grain);
        assert_eq!(v.kind(), "date_grain");
        assert_eq!(v.required_scope(), ScopeRequirement::Profile);
        let json = serde_json::to_string(&v).unwrap();
        let back: PreferenceValue = serde_json::from_str(&json).unwrap();
        assert_eq!(v, back);
    }
}

#[test]
fn output_style_round_trips_and_is_global_scoped() {
    for style in [
        OutputStyle::Table,
        OutputStyle::Compact,
        OutputStyle::Narrative,
    ] {
        let v = PreferenceValue::output_style(style);
        assert_eq!(v.kind(), "output_style");
        assert_eq!(v.required_scope(), ScopeRequirement::Global);
        let json = serde_json::to_string(&v).unwrap();
        let back: PreferenceValue = serde_json::from_str(&json).unwrap();
        assert_eq!(v, back);
    }
}

#[test]
fn default_profile_accepts_name_and_is_global_scoped() {
    let v = PreferenceValue::default_profile("warehouse").unwrap();
    assert_eq!(v.kind(), "default_profile");
    assert_eq!(v.default_profile_name(), Some("warehouse"));
    assert_eq!(v.required_scope(), ScopeRequirement::Global);
}

#[test]
fn default_profile_rejects_bad_names() {
    // Profile names come from `connections.toml`, where a name is an
    // arbitrary map key — config does not reject spaces. A name is rejected
    // only for being empty, too long, or carrying control characters: the
    // same shape `validate_name` applies to database object names. A space
    // is a legitimate character in a profile name, so it is accepted.
    assert!(PreferenceValue::default_profile("").is_err());
    assert!(PreferenceValue::default_profile("name\n").is_err());
    assert!(PreferenceValue::default_profile("name\u{0}").is_err());
    assert!(PreferenceValue::default_profile("x".repeat(MAX_PROFILE_NAME_CHARS + 1)).is_err());
    assert!(PreferenceValue::default_profile("has space").is_ok());
}

#[test]
fn timezone_round_trips_through_serde() {
    let v = PreferenceValue::timezone("America/New_York").unwrap();
    let json = serde_json::to_string(&v).unwrap();
    let back: PreferenceValue = serde_json::from_str(&json).unwrap();
    assert_eq!(v, back);
}
