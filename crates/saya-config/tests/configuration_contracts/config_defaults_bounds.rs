use saya_config::{
    AgentsFile, BudgetLimit, CliOverrides, ConfigError, ConfigFile, ConnectionsFile, JobFile,
    JobLimitsFile, ResolutionInput, SettingSource, resolve,
};
use serde_json::json;

fn finite(value: u64) -> BudgetLimit {
    BudgetLimit::Finite(std::num::NonZeroU64::new(value).unwrap())
}

/// The raw shape a `--coding-max-*` flag hands the merge funnel.
fn toml_integer(value: i64) -> toml::Value {
    toml::Value::Integer(value)
}

fn empty() -> ResolutionInput {
    ResolutionInput::new(ConnectionsFile::default())
}

#[test]
fn core_defaults_are_opt_in_and_bounded() {
    let file = ConfigFile::from_toml("").expect("empty config parses");
    assert_eq!(file.job, JobFile::default());
    assert_eq!(file.limits.job, JobLimitsFile::default());

    let resolved = resolve(empty()).expect("empty config resolves");
    let coding = &resolved.limits.job;
    assert_eq!(coding.logical_requests, finite(2_000));
    assert_eq!(coding.transport_attempts, finite(6_000));
    assert_eq!(coding.tool_calls, finite(10_000));
    assert_eq!(coding.known_tokens, BudgetLimit::Unlimited);
    assert_eq!(coding.elapsed_seconds, finite(604_800));
    assert_eq!(coding.no_progress_turns, 8);
    assert_eq!(coding.repeat_limit, 3);
    assert_eq!(
        resolved
            .provenance
            .get("agents.max_children")
            .unwrap()
            .source,
        SettingSource::Default
    );
}

#[test]
fn max_job_tokens_defaults_to_no_job_wide_ceiling() {
    let resolved = resolve(empty()).expect("empty config resolves");
    assert_eq!(
        resolved.limits.job.job_tokens,
        BudgetLimit::Unlimited,
        "no job-wide ceiling unless configured"
    );
    assert_eq!(
        resolved
            .provenance
            .get("limits.job.job_tokens")
            .unwrap()
            .source,
        SettingSource::Default
    );
}

#[test]
fn max_job_tokens_round_trips_through_merge_and_resolve() {
    let user = ConfigFile::from_toml("[limits.job]\njob_tokens = 500_000\n").unwrap();
    let project = ConfigFile::from_toml("[limits.job]\njob_tokens = 600_000\n").unwrap();
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
    assert_eq!(resolved.limits.job.job_tokens, finite(600_000));
    assert_eq!(
        resolved
            .provenance
            .get("limits.job.job_tokens")
            .unwrap()
            .source,
        SettingSource::ProjectFile
    );
    let diagnostics = serde_json::to_value(resolved.redacted_diagnostics()).unwrap();
    assert_eq!(diagnostics["limits_job_job_tokens"], json!(600_000));
}

/// The `--coding-max-*` flags state `[limits.job]` keys, and each is
/// attributed to the key it overrides.
#[test]
fn job_limit_flags_take_precedence_and_record_their_flag() {
    let user = ConfigFile::from_toml("[agents]\nmax_children = 1\n").unwrap();
    let project = ConfigFile::from_toml("[agents]\nmax_children = 2\n").unwrap();
    let cli = CliOverrides {
        job: JobLimitsFile {
            logical_requests: Some(toml_integer(80)),
            transport_attempts: Some(toml_integer(240)),
            job_tokens: Some(toml_integer(50_000)),
            elapsed_seconds: Some(toml_integer(1_200)),
            ..Default::default()
        },
        agents: AgentsFile {
            max_children: Some(3),
            max_concurrent_model_requests: Some(4),
            ..Default::default()
        },
        trust_project_config: true,
        ..Default::default()
    };
    let resolved = resolve(empty().with_user(user).with_project(project).with_cli(cli)).unwrap();
    assert_eq!(resolved.agents.max_children, 3);
    assert_eq!(resolved.limits.job.job_tokens, finite(50_000));
    for (key, flag) in [
        ("agents.max_children", "--agents-max-children"),
        (
            "agents.max_concurrent_model_requests",
            "--agents-max-concurrent-model-requests",
        ),
        (
            "limits.job.logical_requests",
            "--coding-max-logical-model-requests",
        ),
        (
            "limits.job.transport_attempts",
            "--coding-max-physical-transport-attempts",
        ),
        ("limits.job.job_tokens", "--coding-max-job-tokens"),
        ("limits.job.elapsed_seconds", "--coding-max-elapsed-seconds"),
    ] {
        assert_eq!(
            resolved.provenance.get(key).unwrap().source,
            SettingSource::Cli { flag: flag.into() },
            "wrong CLI provenance for {key}"
        );
    }
}

#[test]
fn agents_model_request_capacity_must_admit_a_coordinator() {
    for budget_mode in ["bounded", "uncapped"] {
        let accepted = resolve(empty().with_user(
            ConfigFile::from_toml(&format!(
                "[limits.job]\nbudget_mode = \"{budget_mode}\"\n[agents]\nmax_concurrent_model_requests = 2\n",
            ))
            .unwrap(),
        ))
        .unwrap();
        assert_eq!(accepted.agents.max_concurrent_model_requests, 2);

        let error = resolve(empty().with_user(
            ConfigFile::from_toml(&format!(
                "[limits.job]\nbudget_mode = \"{budget_mode}\"\n[agents]\nmax_concurrent_model_requests = 1\n",
            ))
            .unwrap(),
        ))
        .unwrap_err();
        assert!(
            matches!(
                error,
                ConfigError::SettingOutOfRange {
                    field: "agents.max_concurrent_model_requests",
                    value: 1,
                    min: 2,
                    max: 4,
                }
            ),
            "expected actionable 2..=4 range in {budget_mode} mode, got {error:?}"
        );
    }
}

#[test]
fn job_unknown_key_is_parse_error() {
    let error = ConfigFile::from_toml("[job]\nenabeld = true\n").unwrap_err();
    assert!(matches!(error, ConfigError::Parse(_)));
    assert!(error.to_string().contains("enabeld"));
}
