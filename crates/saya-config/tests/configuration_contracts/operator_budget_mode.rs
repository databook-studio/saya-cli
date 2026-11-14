use saya_config::{
    BudgetLimit, CliOverrides, CodingBudgetMode, ConfigError, ConfigFile, ConnectionsFile,
    JobLimitsFile, ResolutionInput, SettingSource, resolve,
};

fn empty() -> ResolutionInput {
    ResolutionInput::new(ConnectionsFile::default())
}

fn toml_integer(value: i64) -> toml::Value {
    toml::Value::Integer(value)
}

#[test]
fn operator_uncapped_mode_is_trusted() {
    let user = ConfigFile::from_toml("[limits.job]\nbudget_mode = \"uncapped\"\n")
        .expect("trusted limits.job.budget_mode = uncapped is a recognized setting");
    let project = ConfigFile::from_toml("[limits.job]\nbudget_mode = \"bounded\"\n").unwrap();

    let untrusted = resolve(
        empty()
            .with_user(user.clone())
            .with_project(project.clone()),
    )
    .unwrap();
    assert_eq!(untrusted.limits.job.budget_mode, CodingBudgetMode::Uncapped);
    let effective = serde_json::to_value(untrusted.redacted_diagnostics()).unwrap();
    for field in [
        "limits_job_logical_requests",
        "limits_job_transport_attempts",
        "limits_job_tool_calls",
        "limits_job_known_tokens",
        "limits_job_job_tokens",
        "limits_job_elapsed_seconds",
    ] {
        assert_eq!(effective[field], serde_json::Value::Null, "{field}");
    }
    assert_eq!(untrusted.ignored_project_overrides, ["limits.job"]);
    assert_eq!(
        untrusted
            .provenance
            .get("limits.job.budget_mode")
            .unwrap()
            .source,
        SettingSource::UserFile
    );

    let trusted = resolve(
        empty()
            .with_user(user)
            .with_project(project)
            .with_cli(CliOverrides {
                trust_project_config: true,
                ..Default::default()
            }),
    )
    .unwrap();
    assert_eq!(trusted.limits.job.budget_mode, CodingBudgetMode::Bounded);
    assert!(trusted.ignored_project_overrides.is_empty());
    assert_eq!(
        trusted
            .provenance
            .get("limits.job.budget_mode")
            .unwrap()
            .source,
        SettingSource::ProjectFile
    );
}

#[test]
fn project_uncapped_mode_is_ignored_without_trusted_coding_settings() {
    let project = ConfigFile::from_toml("[limits.job]\nbudget_mode = \"uncapped\"\n").unwrap();
    let resolved = resolve(empty().with_project(project)).unwrap();
    assert_eq!(resolved.limits.job.budget_mode, CodingBudgetMode::Bounded);
    assert_eq!(resolved.ignored_project_overrides, ["limits.job"]);
    assert_eq!(
        resolved
            .provenance
            .get("limits.job.budget_mode")
            .unwrap()
            .source,
        SettingSource::Default
    );
}

#[test]
fn budget_mode_defaults_to_bounded_in_resolved_and_declared_diagnostics() {
    let resolved = resolve(empty()).unwrap();
    assert_eq!(resolved.limits.job.budget_mode, CodingBudgetMode::Bounded);
    assert_eq!(
        resolved
            .provenance
            .get("limits.job.budget_mode")
            .unwrap()
            .source,
        SettingSource::Default
    );
    assert_eq!(
        serde_json::to_value(resolved.redacted_diagnostics()).unwrap()["limits_job_budget_mode"],
        "bounded"
    );
    assert_eq!(
        ConfigFile::default()
            .redacted_diagnostics()
            .limits_job_budget_mode,
        None
    );
}

#[test]
fn unknown_budget_mode_is_rejected() {
    assert!(matches!(
        ConfigFile::from_toml("[limits.job]\nbudget_mode = \"infinite\"\n"),
        Err(ConfigError::Parse(_))
    ));
}

/// `uncapped` states "no ceiling at all", so a dimension beside it is refused
/// by the key that carries it.
#[test]
fn uncapped_mode_rejects_explicit_aggregate_numeric_limits() {
    for (field, value) in [
        ("logical_requests", "1"),
        ("transport_attempts", "240"),
        ("tool_calls", "1"),
        ("known_tokens", "1"),
        ("job_tokens", "1"),
        ("elapsed_seconds", "1"),
    ] {
        let config = ConfigFile::from_toml(&format!(
            "[limits.job]\nbudget_mode = \"uncapped\"\n{field} = {value}\n"
        ))
        .unwrap();
        let error = resolve(empty().with_user(config)).unwrap_err();
        let expected = format!("limits.job.{field}");
        assert!(
            matches!(&error, ConfigError::JobBudgetModeConflict { setting } if *setting == expected.as_str()),
            "uncapped mode must refuse explicitly configured {field}: {error}"
        );
    }
}

#[test]
fn uncapped_mode_rejects_aggregate_cli_overrides() {
    let user = ConfigFile::from_toml("[limits.job]\nbudget_mode = \"uncapped\"\n").unwrap();
    for (field, job) in [
        (
            "logical_requests",
            JobLimitsFile {
                logical_requests: Some(toml_integer(1)),
                ..Default::default()
            },
        ),
        (
            "transport_attempts",
            JobLimitsFile {
                transport_attempts: Some(toml_integer(240)),
                ..Default::default()
            },
        ),
        (
            "job_tokens",
            JobLimitsFile {
                job_tokens: Some(toml_integer(1)),
                ..Default::default()
            },
        ),
        (
            "elapsed_seconds",
            JobLimitsFile {
                elapsed_seconds: Some(toml_integer(1)),
                ..Default::default()
            },
        ),
    ] {
        let error = resolve(empty().with_user(user.clone()).with_cli(CliOverrides {
            job,
            ..Default::default()
        }))
        .unwrap_err();
        let expected = format!("limits.job.{field}");
        assert!(
            matches!(&error, ConfigError::JobBudgetModeConflict { setting } if *setting == expected.as_str()),
            "uncapped mode must refuse CLI override for {field}: {error}"
        );
    }
}

/// An uncapped job has no ceilings to read, so every dimension resolves to
/// `Unlimited` rather than to the bounded defaults.
#[test]
fn uncapped_mode_resolves_every_dimension_to_unlimited() {
    let resolved =
        resolve(empty().with_user(
            ConfigFile::from_toml("[limits.job]\nbudget_mode = \"uncapped\"\n").unwrap(),
        ))
        .unwrap();
    let coding = &resolved.limits.job;
    assert_eq!(coding.logical_requests, BudgetLimit::Unlimited);
    assert_eq!(coding.transport_attempts, BudgetLimit::Unlimited);
    assert_eq!(coding.tool_calls, BudgetLimit::Unlimited);
    assert_eq!(coding.known_tokens, BudgetLimit::Unlimited);
    assert_eq!(coding.job_tokens, BudgetLimit::Unlimited);
    assert_eq!(coding.elapsed_seconds, BudgetLimit::Unlimited);
}
