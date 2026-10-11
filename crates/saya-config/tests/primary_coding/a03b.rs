use std::collections::BTreeSet;

use saya_config::{ConfigFile, ConnectionsFile, ResolutionInput, resolve, user_facing_keys};
use serde_json::{Value, json};

fn empty() -> ResolutionInput {
    ResolutionInput::new(ConnectionsFile::default())
}

fn assert_json_fields(object: &Value, expected: &[(&str, Value)]) {
    for (field, value) in expected {
        assert_eq!(&object[*field], value, "wrong diagnostic value for {field}");
        if value.is_boolean() {
            assert!(
                object[*field].is_boolean(),
                "{field} must be a JSON boolean"
            );
        } else if value.is_number() {
            assert!(object[*field].is_number(), "{field} must be a JSON number");
        }
    }
}

#[test]
fn coding_diagnostics_preserve_raw_effective_types_and_provenance_keys() {
    let absent = ConfigFile::from_toml("").expect("empty config parses");
    let raw_absent = absent.redacted_diagnostics();
    assert!(
        raw_absent.coding_enabled.is_none()
            && raw_absent.coding_max_children.is_none()
            && raw_absent.coding_max_concurrent_model_requests.is_none()
            && raw_absent.coding_max_concurrent_commands.is_none()
            && raw_absent.coding_max_logical_model_requests.is_none()
            && raw_absent.coding_max_physical_transport_attempts.is_none()
            && raw_absent.coding_max_tool_calls.is_none()
            && raw_absent.coding_max_known_tokens.is_none()
            && raw_absent.coding_max_elapsed_seconds.is_none()
    );
    let raw_json = serde_json::to_value(raw_absent).unwrap();
    let missing = [
        ("coding_enabled", Value::Null),
        ("coding_max_children", Value::Null),
        ("coding_max_concurrent_model_requests", Value::Null),
        ("coding_max_concurrent_commands", Value::Null),
        ("coding_max_logical_model_requests", Value::Null),
        ("coding_max_physical_transport_attempts", Value::Null),
        ("coding_max_tool_calls", Value::Null),
        ("coding_max_known_tokens", Value::Null),
        ("coding_max_elapsed_seconds", Value::Null),
    ];
    assert_json_fields(&raw_json, &missing);

    let resolved_defaults = resolve(empty()).expect("default config resolves");
    let effective_defaults =
        serde_json::to_value(resolved_defaults.redacted_diagnostics()).unwrap();
    let defaults = [
        ("coding_enabled", json!(false)),
        ("coding_max_children", json!(3)),
        ("coding_max_concurrent_model_requests", json!(4)),
        ("coding_max_concurrent_commands", json!(2)),
        ("coding_max_logical_model_requests", json!(80)),
        ("coding_max_physical_transport_attempts", json!(240)),
        ("coding_max_tool_calls", json!(300)),
        ("coding_max_known_tokens", json!(400_000)),
        ("coding_max_elapsed_seconds", json!(1_200)),
    ];
    assert_json_fields(&effective_defaults, &defaults);

    let file = ConfigFile::from_toml(
        "[coding]\nenabled = true\nmax_children = 2\nmax_concurrent_model_requests = 3\nmax_concurrent_commands = 1\nmax_logical_model_requests = 40\nmax_physical_transport_attempts = 60\nmax_tool_calls = 123\nmax_known_tokens = 23456\nmax_elapsed_seconds = 900\n\n[coding.extensions.lookup]\nprotocol = 'mcp'\nprogram = 'extension-server'\nargv = ['private-extension-arg']\n\n[coding.check_profiles.ci]\ncommand_id = 'cargo-test'\nprogram = 'cargo'\nargv = ['test']\ntimeout_seconds = 90\nenvironment_profile = 'confined-v1'\n",
    )
    .expect("coding values parse");
    let raw_user = serde_json::to_value(file.redacted_diagnostics()).unwrap();
    let resolved_user = resolve(empty().with_user(file)).expect("user config resolves");
    let effective_user = serde_json::to_value(resolved_user.redacted_diagnostics()).unwrap();
    let user_values = [
        ("coding_enabled", json!(true)),
        ("coding_max_children", json!(2)),
        ("coding_max_concurrent_model_requests", json!(3)),
        ("coding_max_concurrent_commands", json!(1)),
        ("coding_max_logical_model_requests", json!(40)),
        ("coding_max_physical_transport_attempts", json!(60)),
        ("coding_max_tool_calls", json!(123)),
        ("coding_max_known_tokens", json!(23_456)),
        ("coding_max_elapsed_seconds", json!(900)),
        (
            "coding_extensions",
            json!({"lookup": {
                "protocol": "mcp",
                "program": "extension-server",
                "argv_count": 1,
                "language": null,
                "tool_policy_count": 0,
            }}),
        ),
        (
            "coding_check_profiles",
            json!({"ci": {
                "command_id": "cargo-test",
                "program": "cargo",
                "argv_count": 1,
                "timeout_seconds": 90,
                "toolchain_reference": null,
                "environment_profile_reference": "confined-v1",
            }}),
        ),
    ];
    assert_json_fields(&raw_user, &user_values);
    assert_json_fields(&effective_user, &user_values);

    let expected_keys = [
        ("coding_extensions", "coding.extensions"),
        ("coding_enabled", "coding.enabled"),
        ("coding_max_children", "coding.max_children"),
        (
            "coding_max_concurrent_model_requests",
            "coding.max_concurrent_model_requests",
        ),
        (
            "coding_max_concurrent_commands",
            "coding.max_concurrent_commands",
        ),
        (
            "coding_max_logical_model_requests",
            "coding.max_logical_model_requests",
        ),
        (
            "coding_max_physical_transport_attempts",
            "coding.max_physical_transport_attempts",
        ),
        ("coding_max_tool_calls", "coding.max_tool_calls"),
        ("coding_max_known_tokens", "coding.max_known_tokens"),
        ("coding_max_elapsed_seconds", "coding.max_elapsed_seconds"),
        ("coding_check_profiles", "coding.check_profiles"),
    ];
    let actual_keys: BTreeSet<_> = user_facing_keys()
        .iter()
        .copied()
        .filter(|(field, _)| field.starts_with("coding_"))
        .collect();
    assert_eq!(actual_keys, expected_keys.into_iter().collect());
    for (_, dotted_key) in expected_keys {
        assert!(
            resolved_user.provenance.get(dotted_key).is_some(),
            "missing provenance for {dotted_key}"
        );
    }
}

#[test]
fn coding_effective_diagnostics_follow_untrusted_project_reversion() {
    let user = ConfigFile::from_toml("[coding]\nmax_children = 2\n").unwrap();
    let project = ConfigFile::from_toml("[coding]\nmax_children = 1\n").unwrap();
    let raw_project = serde_json::to_value(project.redacted_diagnostics()).unwrap();
    assert_eq!(raw_project["coding_max_children"], json!(1));

    let resolved = resolve(empty().with_user(user).with_project(project)).unwrap();
    assert_eq!(resolved.ignored_project_overrides, ["coding"]);
    let effective = serde_json::to_value(resolved.redacted_diagnostics()).unwrap();
    assert_eq!(effective["coding_max_children"], json!(2));
}
