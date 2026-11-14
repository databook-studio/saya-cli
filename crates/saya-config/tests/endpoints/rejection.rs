//! Typed rejections at parse and resolution: duplicate names are a typed
//! error rather than last-wins, malformed and missing names fail loudly,
//! the pool itself is bounded, and unknown endpoint keys are denied.

use saya_config::{ConfigError, ConfigFile};

use super::resolve_with_user;

#[test]
fn duplicate_endpoint_names_are_rejected_not_last_wins() {
    let error = resolve_with_user(
        "[[ai.endpoints]]\nname = 'planner'\nmodel = 'a'\n\
         [[ai.endpoints]]\nname = 'planner'\nmodel = 'b'\n",
    )
    .unwrap_err();
    assert!(
        matches!(error, ConfigError::DuplicateEndpointName(_)),
        "duplicates must be a typed error, not last-wins: {error:?}"
    );
    assert!(error.to_string().contains("planner"));
}

#[test]
fn a_malformed_endpoint_name_is_rejected() {
    let error = resolve_with_user("[[ai.endpoints]]\nname = 'bad name'\n").unwrap_err();
    assert!(matches!(
        error,
        ConfigError::InvalidEndpointName {
            field: "ai.endpoints",
            ..
        }
    ));
    assert!(error.to_string().contains("bad name"));
}

#[test]
fn the_endpoint_pool_is_bounded() {
    let mut toml = String::new();
    for index in 0..9 {
        toml.push_str(&format!("[[ai.endpoints]]\nname = 'ep{index}'\n"));
    }
    let error = resolve_with_user(&toml).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::SettingAboveMaximum {
            field: "ai.endpoints",
            ..
        }
    ));
}

#[test]
fn unknown_keys_inside_an_endpoint_are_rejected() {
    let error = ConfigFile::from_toml("[[ai.endpoints]]\nname = 'p'\nbaz = 'x'\n").unwrap_err();
    assert!(
        error.to_string().contains("baz"),
        "deny_unknown_fields must name the key: {error}"
    );
}

#[test]
fn an_endpoint_without_a_name_is_rejected_at_parse() {
    let error = ConfigFile::from_toml("[[ai.endpoints]]\nmodel = 'm'\n").unwrap_err();
    assert!(
        error.to_string().contains("name"),
        "the name is the map key; its absence must fail loudly: {error}"
    );
}
