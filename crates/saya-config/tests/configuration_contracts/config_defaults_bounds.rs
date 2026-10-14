use saya_config::{
    CliOverrides, CodingFile, ConfigError, ConfigFile, ConnectionsFile, ResolutionInput,
    SettingSource, resolve,
};
use serde_json::json;

fn empty() -> ResolutionInput {
    ResolutionInput::new(ConnectionsFile::default())
}

#[test]
fn core_defaults_are_opt_in_and_bounded() {
    let file = ConfigFile::from_toml("").expect("empty config parses");
    assert_eq!(file.coding, CodingFile::default());

    let resolved = resolve(empty()).expect("empty config resolves");
    let coding = &resolved.coding;
    assert!(!coding.enabled);
    assert_eq!(coding.max_children, 3);
    assert_eq!(coding.max_concurrent_model_requests, 4);
    assert_eq!(coding.max_concurrent_commands, 2);
    assert_eq!(coding.max_logical_model_requests, 80);
    assert_eq!(coding.max_physical_transport_attempts, 240);
    assert_eq!(coding.max_tool_calls, 300);
    assert_eq!(coding.max_known_tokens, 400_000);
    assert_eq!(coding.max_elapsed_seconds, 1_200);
    assert!(coding.check_profiles.is_empty());
    assert_eq!(
        resolved
            .provenance
            .get("coding.max_children")
            .unwrap()
            .source,
        SettingSource::Default
    );
}

#[test]
fn max_job_tokens_defaults_to_no_job_wide_ceiling() {
    let resolved = resolve(empty()).expect("empty config resolves");
    assert_eq!(
        resolved.coding.max_job_tokens, None,
        "no job-wide ceiling unless configured"
    );
    assert_eq!(
        resolved
            .provenance
            .get("coding.max_job_tokens")
            .unwrap()
            .source,
        SettingSource::Default
    );
}

#[test]
fn max_job_tokens_round_trips_through_merge_and_resolve() {
    let user = ConfigFile::from_toml("[coding]\nmax_job_tokens = 500_000\n").unwrap();
    let project = ConfigFile::from_toml("[coding]\nmax_job_tokens = 600_000\n").unwrap();
    let resolved = resolve(
        empty()
            .with_user(user)
            .with_project(project)
            .with_cli(CliOverrides {
                trust_project_config: true,
                ..Default::default()
            }),
    )
    .unwrap();
    assert_eq!(resolved.coding.max_job_tokens, Some(600_000));
    assert_eq!(
        resolved
            .provenance
            .get("coding.max_job_tokens")
            .unwrap()
            .source,
        SettingSource::ProjectFile
    );
    let diagnostics = serde_json::to_value(resolved.redacted_diagnostics()).unwrap();
    assert_eq!(diagnostics["coding_max_job_tokens"], json!(600_000));
}

#[test]
fn coding_config_precedence_and_invalid_bounds() {
    let user = ConfigFile::from_toml("[coding]\nmax_children = 1\n").unwrap();
    let project = ConfigFile::from_toml("[coding]\nmax_children = 2\n").unwrap();
    let cli = CliOverrides {
        coding: CodingFile {
            extensions: None,
            enabled: Some(false),
            max_children: Some(3),
            max_concurrent_model_requests: Some(4),
            max_concurrent_commands: Some(2),
            max_logical_model_requests: Some(80),
            max_physical_transport_attempts: Some(240),
            max_tool_calls: Some(300),
            max_known_tokens: Some(400_000),
            max_job_tokens: Some(50_000),
            max_elapsed_seconds: Some(1_200),
            check_profiles: None,
        },
        trust_project_config: true,
        ..Default::default()
    };
    let resolved = resolve(empty().with_user(user).with_project(project).with_cli(cli)).unwrap();
    assert_eq!(resolved.coding.max_children, 3);
    assert_eq!(resolved.coding.max_job_tokens, Some(50_000));
    for (key, flag) in [
        ("coding.enabled", "--coding"),
        ("coding.max_children", "--coding-max-children"),
        (
            "coding.max_concurrent_model_requests",
            "--coding-max-concurrent-model-requests",
        ),
        (
            "coding.max_concurrent_commands",
            "--coding-max-concurrent-commands",
        ),
        (
            "coding.max_logical_model_requests",
            "--coding-max-logical-model-requests",
        ),
        (
            "coding.max_physical_transport_attempts",
            "--coding-max-physical-transport-attempts",
        ),
        ("coding.max_tool_calls", "--coding-max-tool-calls"),
        ("coding.max_known_tokens", "--coding-max-known-tokens"),
        ("coding.max_job_tokens", "--coding-max-job-tokens"),
        ("coding.max_elapsed_seconds", "--coding-max-elapsed-seconds"),
    ] {
        assert_eq!(
            resolved.provenance.get(key).unwrap().source,
            SettingSource::Cli { flag: flag.into() },
            "wrong CLI provenance for {key}"
        );
    }

    for (field, value, min, max) in [
        ("max_children", 0, 1, 3),
        ("max_children", 4, 1, 3),
        ("max_concurrent_model_requests", 0, 1, 4),
        ("max_concurrent_model_requests", 5, 1, 4),
        ("max_concurrent_commands", 0, 1, 2),
        ("max_concurrent_commands", 3, 1, 2),
        ("max_logical_model_requests", 0, 1, 80),
        ("max_logical_model_requests", 81, 1, 80),
        ("max_physical_transport_attempts", 0, 1, 240),
        ("max_physical_transport_attempts", 241, 1, 240),
        ("max_tool_calls", 0, 1, 300),
        ("max_tool_calls", 301, 1, 300),
        ("max_known_tokens", 0, 1, 400_000),
        ("max_known_tokens", 400_001, 1, 400_000),
        ("max_job_tokens", 0, 1, 100_000_000),
        ("max_job_tokens", 100_000_001, 1, 100_000_000),
        ("max_elapsed_seconds", 0, 1, 1_200),
        ("max_elapsed_seconds", 1_201, 1, 1_200),
    ] {
        let error =
            resolve(empty().with_user(
                ConfigFile::from_toml(&format!("[coding]\n{field} = {value}\n")).unwrap(),
            ))
            .unwrap_err();
        assert!(
            matches!(
                error,
                ConfigError::SettingOutOfRange {
                    field: actual,
                    value: actual_value,
                    min: actual_min,
                    max: actual_max,
                } if actual == format!("coding.{field}")
                    && actual_value == value
                    && actual_min == min
                    && actual_max == max
            ),
            "expected range error for coding.{field} = {value}, got {error:?}"
        );
    }

    let error = resolve(
        empty().with_user(
            ConfigFile::from_toml(
                "[coding]\nmax_logical_model_requests = 2\nmax_physical_transport_attempts = 1\n",
            )
            .unwrap(),
        ),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ConfigError::SettingOutOfRange {
            field: "coding.max_physical_transport_attempts",
            value: 1,
            min: 2,
            max: 240,
        }
    ));
}

#[test]
fn coding_unknown_key_is_parse_error() {
    let error = ConfigFile::from_toml("[coding]\nenabeld = true\n").unwrap_err();
    assert!(matches!(error, ConfigError::Parse(_)));
    assert!(error.to_string().contains("enabeld"));
}
