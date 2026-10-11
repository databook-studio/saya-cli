use saya_config::{ConfigError, ConfigFile, ConnectionsFile, ResolutionInput, resolve};

const ARG_SENTINEL: &str = "c03b-private-argv-sentinel";
const PROFILE_FILE: &str = r#"
[coding.check_profiles.ci]
command_id = "cargo-test"
program = "cargo"
argv = ["test", "--locked", "c03b-private-argv-sentinel"]
timeout_seconds = 90
toolchain = "rust-stable"
environment_profile = "confined-v1"
diagnostic_format = "rustc_json"
"#;

fn resolved(file: ConfigFile) -> saya_config::ResolvedConfig {
    resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(file)).unwrap()
}

#[test]
fn profile_loads_as_fixed_data_and_diagnostics_hide_argv() {
    let file = ConfigFile::from_toml(PROFILE_FILE)
        .expect("the coding check profile table is part of the config schema");
    let raw: serde_json::Value = serde_json::to_value(file.redacted_diagnostics()).unwrap();
    let raw_profile = &raw["coding_check_profiles"]["ci"];
    assert_eq!(raw_profile["command_id"], "cargo-test");
    assert_eq!(raw_profile["program"], "cargo");
    assert_eq!(raw_profile["argv_count"], 3);
    assert_eq!(raw_profile["timeout_seconds"], 90);
    assert_eq!(raw_profile["diagnostic_format"], "rustc_json");
    assert!(!raw.to_string().contains(ARG_SENTINEL));
    assert!(!format!("{file:?}").contains(ARG_SENTINEL));
    assert!(
        saya_config::user_facing_keys()
            .contains(&("coding_check_profiles", "coding.check_profiles"))
    );

    let resolved = resolved(file);
    assert_eq!(
        resolved
            .coding
            .select_check_profile("ci")
            .unwrap()
            .diagnostic_format(),
        Some(saya_config::CheckDiagnosticFormat::RustcJson)
    );
    let effective: serde_json::Value =
        serde_json::to_value(resolved.redacted_diagnostics()).unwrap();
    assert_eq!(effective["coding_check_profiles"]["ci"]["argv_count"], 3);
    assert_eq!(
        effective["coding_check_profiles"]["ci"]["diagnostic_format"],
        "rustc_json"
    );
    assert!(!effective.to_string().contains(ARG_SENTINEL));
    assert!(!format!("{:?}", resolved.coding).contains(ARG_SENTINEL));
}

#[test]
fn diagnostic_format_is_optional_and_unknown_values_are_not_echoed() {
    let file = ConfigFile::from_toml(&profile_toml("ci", "cargo", &[], 10)).unwrap();
    let raw: serde_json::Value = serde_json::to_value(file.redacted_diagnostics()).unwrap();
    assert!(
        raw["coding_check_profiles"]["ci"]
            .get("diagnostic_format")
            .is_none()
    );
    let resolved = resolved(file);
    assert_eq!(
        resolved
            .coding
            .select_check_profile("ci")
            .unwrap()
            .diagnostic_format(),
        None
    );
    let effective: serde_json::Value =
        serde_json::to_value(resolved.redacted_diagnostics()).unwrap();
    assert!(
        effective["coding_check_profiles"]["ci"]
            .get("diagnostic_format")
            .is_none()
    );

    let untrusted = "model-picked-unsupported-format";
    let invalid = PROFILE_FILE.replace(
        "diagnostic_format = \"rustc_json\"",
        &format!("diagnostic_format = \"{untrusted}\""),
    );
    let error = ConfigFile::from_toml(&invalid).unwrap_err();
    assert!(!error.to_string().contains(untrusted));
}

#[test]
fn cargo_json_format_is_explicit_profile_data_without_changing_argv() {
    let toml = PROFILE_FILE.replace("rustc_json", "cargo_json");
    let file = ConfigFile::from_toml(&toml).unwrap();
    let raw: serde_json::Value = serde_json::to_value(file.redacted_diagnostics()).unwrap();
    assert_eq!(
        raw["coding_check_profiles"]["ci"]["diagnostic_format"],
        "cargo_json"
    );
    let resolved = resolved(file);
    let profile = resolved.coding.select_check_profile("ci").unwrap();
    assert_eq!(
        profile.diagnostic_format(),
        Some(saya_config::CheckDiagnosticFormat::CargoJson)
    );
    assert_eq!(profile.argv(), ["test", "--locked", ARG_SENTINEL]);
    let effective: serde_json::Value =
        serde_json::to_value(resolved.redacted_diagnostics()).unwrap();
    assert_eq!(
        effective["coding_check_profiles"]["ci"]["diagnostic_format"],
        "cargo_json"
    );
    assert!(!effective.to_string().contains(ARG_SENTINEL));
}

#[test]
fn malformed_profile_fields_fail_during_resolution_without_echoing_values() {
    let bad = r#"
[coding.check_profiles.ci]
command_id = "cargo-test"
program = "../cargo"
argv = ["c03b-secret-argument"]
timeout_seconds = 901
environment_profile = "confined-v1"
"#;
    let file = ConfigFile::from_toml(bad).expect("profile data deserializes before validation");
    let error = resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(file))
        .expect_err("path-shaped program and over-limit timeout are refused");
    let message = error.to_string();
    assert!(
        message.contains("check_profiles"),
        "field is actionable: {message}"
    );
    assert!(!message.contains("c03b-secret-argument"));
    assert!(!message.contains("../cargo"));
}

#[test]
fn profile_selection_is_exact_and_unknown_ids_are_not_echoed() {
    let resolved = resolved(ConfigFile::from_toml(PROFILE_FILE).unwrap());
    let profile = resolved.coding.select_check_profile("ci").unwrap();
    assert_eq!(profile.profile_id(), "ci");
    assert_eq!(profile.command_id(), "cargo-test");
    assert_eq!(profile.program(), "cargo");
    assert_eq!(profile.argv(), ["test", "--locked", ARG_SENTINEL]);
    assert_eq!(profile.timeout_seconds(), 90);
    assert_eq!(profile.toolchain_reference(), Some("rust-stable"));
    assert_eq!(profile.environment_profile_reference(), "confined-v1");

    let untrusted = "model-selected-profile-that-does-not-exist";
    let error = resolved.coding.select_check_profile(untrusted).unwrap_err();
    assert!(matches!(error, ConfigError::UnknownCheckProfile));
    assert!(!error.to_string().contains(untrusted));
}

fn profile_toml(id: &str, program: &str, argv: &[String], timeout_seconds: u64) -> String {
    let mut result = format!(
        "[coding.check_profiles.\"{id}\"]\ncommand_id='test'\nprogram={}\ntimeout_seconds={timeout_seconds}\nenvironment_profile='confined'\nargv=[",
        serde_json::to_string(program).unwrap()
    );
    for (index, argument) in argv.iter().enumerate() {
        if index > 0 {
            result.push(',');
        }
        result.push_str(&serde_json::to_string(argument).unwrap());
    }
    result.push_str("]\n");
    result
}

fn invalid_profile_error(toml: &str) -> ConfigError {
    let file = ConfigFile::from_toml(toml).expect("the invalid profile fixture is valid TOML");
    resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(file))
        .expect_err("invalid profile data is refused")
}

#[test]
fn profile_bounds_reject_path_nul_timeout_and_argv_growth() {
    let cases = [
        profile_toml("ci", "../cargo", &[], 10),
        profile_toml("ci", "cargo", &["has\0nul".to_owned()], 10),
        profile_toml("ci", "cargo", &["x".repeat(16 * 1024 + 1)], 10),
        profile_toml("ci", "cargo", &vec!["x".repeat(16 * 1024); 9], 10),
        profile_toml("ci", "cargo", &vec!["x".into(); 129], 10),
        profile_toml("ci", "cargo", &[], 0),
        profile_toml("ci", "cargo", &[], 901),
    ];
    for toml in cases {
        let error = invalid_profile_error(&toml);
        assert!(
            matches!(error, ConfigError::InvalidCheckProfile { .. }),
            "{error}"
        );
    }
}

#[test]
fn profile_count_is_bounded() {
    let mut toml = String::new();
    for index in 0..17 {
        toml.push_str(&profile_toml(&format!("p{index}"), "cargo", &[], 10));
    }
    let error = invalid_profile_error(&toml);
    assert!(matches!(error, ConfigError::TooManyCheckProfiles { .. }));
}
