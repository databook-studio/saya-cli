//! The from-JSON admission gate: a valid record round-trips, and every wire
//! refusal fires — unknown version, unknown field, unrepresentable source,
//! and the record byte cap that fires before any parsing.

use super::*;

// ---------------------------------------------------------------------------
// The bounds pin their scale, so a loosened bound is visible in the diff
// ---------------------------------------------------------------------------

#[test]
fn the_bounds_pin_their_scale() {
    assert_eq!(PROJECT_PREFERENCE_VERSION, 1);
    assert_eq!(MAX_PROJECT_PREFERENCES, 64);
    assert_eq!(MAX_PREFERENCE_TEXT_BYTES, 1024);
    assert_eq!(MAX_PREFERENCE_RECALL_BYTES, 16 * 1024);
    assert_eq!(MAX_PREFERENCE_RECORD_BYTES, 4 * 1024);
}

// ---------------------------------------------------------------------------
// Valid records round-trip; unknown versions and unknown fields are refused
// ---------------------------------------------------------------------------

fn wire_version(version: u32) -> String {
    let id = pref_id(0);
    let wt = worktree();
    format!(
        r#"{{"version":{version},"id":"{id}","worktree":"{wt}","text":"t","source":"user_explicit","created_unix_ms":1700000000000,"updated_unix_ms":1700000000000}}"#,
        id = id.as_str(),
        wt = wt.as_str(),
    )
}

fn wire_extra(extra: &str) -> String {
    let id = pref_id(0);
    let wt = worktree();
    format!(
        r#"{{"version":1,"id":"{id}","worktree":"{wt}","text":"t","source":"user_explicit","created_unix_ms":1700000000000,"updated_unix_ms":1700000000000{extra}}}"#,
        id = id.as_str(),
        wt = wt.as_str(),
    )
}

fn wire_source(source: &str) -> String {
    let id = pref_id(0);
    let wt = worktree();
    format!(
        r#"{{"version":1,"id":"{id}","worktree":"{wt}","text":"t","source":"{source}","created_unix_ms":1700000000000,"updated_unix_ms":1700000000000}}"#,
        id = id.as_str(),
        wt = wt.as_str(),
    )
}

#[test]
fn a_valid_record_round_trips() {
    let record = record();
    let bytes = serde_json::to_vec(&record).unwrap();
    assert_eq!(ProjectPreference::from_json_bytes(&bytes).unwrap(), record);
}

#[test]
fn unsupported_versions_are_refused() {
    for version in [0u32, 2] {
        assert_eq!(
            ProjectPreference::from_json_bytes(wire_version(version).as_bytes()).unwrap_err(),
            PreferenceError::UnsupportedVersion(version)
        );
    }
}

#[test]
fn an_unknown_field_is_refused() {
    for extra in [r#","command":"rm -rf /""#, r#","grants":["sql:execute"]"#] {
        assert_eq!(
            ProjectPreference::from_json_bytes(wire_extra(extra).as_bytes()).unwrap_err(),
            PreferenceError::Malformed
        );
    }
}

#[test]
fn assistant_inferred_sources_are_unrepresentable() {
    for source in ["assistant_inferred", "generated"] {
        assert_eq!(
            ProjectPreference::from_json_bytes(wire_source(source).as_bytes()).unwrap_err(),
            PreferenceError::Malformed
        );
    }
}

// ---------------------------------------------------------------------------
// The record byte cap fires before any parsing
// ---------------------------------------------------------------------------

#[test]
fn wire_records_over_the_record_cap_are_refused_before_parsing() {
    let text = "a".repeat(MAX_PREFERENCE_RECORD_BYTES + 1);
    let json = wire_with(pref_id(0).as_str(), worktree().as_str(), &text);
    assert_eq!(
        ProjectPreference::from_json_bytes(json.as_bytes()).unwrap_err(),
        PreferenceError::Oversize(json.len())
    );
}
