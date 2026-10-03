use saya_config::{
    BudgetLimit, CliOverrides, ConfigError, ConfigFile, ConnectionsFile, ResolutionInput,
    ResolvedInvestigationBudgets, resolve,
};

fn finite_limit(value: u64) -> BudgetLimit {
    BudgetLimit::Finite(std::num::NonZeroU64::new(value).unwrap())
}

type BudgetSelector = fn(&ResolvedInvestigationBudgets) -> BudgetLimit;

#[test]
fn cli_values_override_every_other_source() {
    let input = ResolutionInput::new(ConnectionsFile::default())
        .with_user(ConfigFile::from_toml("[ai]\nmodel = 'user'\n").unwrap())
        .with_project(ConfigFile::from_toml("[ai]\nmodel = 'project'\n").unwrap())
        .with_env_file([("SAYA_AI_MODEL", "env-file")])
        .with_process_env([("SAYA_AI_MODEL", "process")])
        .with_cli(CliOverrides {
            model: Some("cli".into()),
            ..Default::default()
        });

    assert_eq!(resolve(input).unwrap().ai.model, "cli");
}

#[test]
fn profile_selection_prefers_flag_then_environment_then_config() {
    let connections = ConnectionsFile::from_toml(
        "[profiles.first]\ntype = 'duckdb'\npath = ':memory:'\n\
         [profiles.second]\ntype = 'duckdb'\npath = ':memory:'\n",
    )
    .unwrap();
    let config = ConfigFile::from_toml("default_profile = 'first'").unwrap();
    let env = ResolutionInput::new(connections)
        .with_user(config)
        .with_process_env([("SAYA_PROFILE", "second")]);
    assert_eq!(
        resolve(env).unwrap().profile_name.as_deref(),
        Some("second")
    );
}

#[test]
fn process_environment_overrides_the_explicit_env_file() {
    let input = ResolutionInput::new(ConnectionsFile::default())
        .with_env_file([("SAYA_AI_MODEL", "env-file")])
        .with_process_env([("SAYA_AI_MODEL", "process")]);
    assert_eq!(resolve(input).unwrap().ai.model, "process");
}

#[test]
fn multiple_profiles_require_explicit_selection() {
    let connections = ConnectionsFile::from_toml(
        "[profiles.first]\ntype = 'duckdb'\npath = ':memory:'\n\
         [profiles.second]\ntype = 'duckdb'\npath = ':memory:'\n",
    )
    .unwrap();
    assert!(resolve(ResolutionInput::new(connections)).is_err());
}

#[test]
fn a_single_profile_is_selected_without_an_explicit_default() {
    let connections =
        ConnectionsFile::from_toml("[profiles.local]\ntype = 'duckdb'\npath = ':memory:'\n")
            .unwrap();
    assert_eq!(
        resolve(ResolutionInput::new(connections))
            .unwrap()
            .profile_name
            .as_deref(),
        Some("local")
    );
}

#[test]
fn interactive_investigation_budgets_resolve_validated_policy() {
    let defaults = resolve(ResolutionInput::new(ConnectionsFile::default()))
        .unwrap()
        .investigation_budgets;
    assert_eq!(
        defaults,
        ResolvedInvestigationBudgets {
            logical_answering_requests: finite_limit(24),
            requested_tool_calls: finite_limit(64),
            elapsed_seconds: finite_limit(300),
            known_reported_tokens: finite_limit(250_000),
        }
    );

    let finite = ConfigFile::from_toml(
        "[ai]\ninvestigation_max_logical_answering_requests = 8\n\
         investigation_max_requested_tool_calls = 12\n\
         investigation_max_elapsed_seconds = 45\n\
         investigation_max_known_reported_tokens = 9000\n",
    )
    .unwrap();
    let resolved =
        resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(finite)).unwrap();
    assert_eq!(
        resolved.investigation_budgets,
        ResolvedInvestigationBudgets {
            logical_answering_requests: finite_limit(8),
            requested_tool_calls: finite_limit(12),
            elapsed_seconds: finite_limit(45),
            known_reported_tokens: finite_limit(9000),
        }
    );

    let unlimited_fields: [(&str, BudgetSelector); 4] = [
        ("investigation_max_logical_answering_requests", |budgets| {
            budgets.logical_answering_requests
        }),
        ("investigation_max_requested_tool_calls", |budgets| {
            budgets.requested_tool_calls
        }),
        ("investigation_max_elapsed_seconds", |budgets| {
            budgets.elapsed_seconds
        }),
        ("investigation_max_known_reported_tokens", |budgets| {
            budgets.known_reported_tokens
        }),
    ];
    for (field, select) in unlimited_fields {
        let config = ConfigFile::from_toml(&format!("[ai]\n{field} = 'unlimited'\n")).unwrap();
        let budgets = resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(config))
            .unwrap()
            .investigation_budgets;
        assert_eq!(select(&budgets), BudgetLimit::Unlimited, "{field}");
    }

    for (value, expected_field) in [
        ("0", "investigation_max_logical_answering_requests"),
        ("-1", "investigation_max_requested_tool_calls"),
        ("'not-unlimited'", "investigation_max_elapsed_seconds"),
        (
            "'top-secret-token'",
            "investigation_max_known_reported_tokens",
        ),
        ("''", "investigation_max_known_reported_tokens"),
        (
            "'18446744073709551616'",
            "investigation_max_logical_answering_requests",
        ),
        ("'unlimited '", "investigation_max_requested_tool_calls"),
        ("1.5", "investigation_max_logical_answering_requests"),
        ("true", "investigation_max_requested_tool_calls"),
        ("[]", "investigation_max_elapsed_seconds"),
    ] {
        let config = ConfigFile::from_toml(&format!("[ai]\n{expected_field} = {value}\n")).unwrap();
        let error = resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(config))
            .unwrap_err();
        assert!(matches!(
            &error,
            ConfigError::InvalidInvestigationBudget { field } if *field == expected_field
        ));
        assert!(!error.to_string().contains(value));
    }

    assert!(matches!(
        ConfigFile::from_toml(
            "[ai]\ninvestigation_max_logical_answering_requests = 9223372036854775808\n"
        ),
        Err(ConfigError::Parse(_))
    ));
    let largest_toml_integer = ConfigFile::from_toml(
        "[ai]\ninvestigation_max_logical_answering_requests = 9223372036854775807\n",
    )
    .unwrap();
    let largest_toml_integer =
        resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(largest_toml_integer))
            .unwrap();
    assert_eq!(
        largest_toml_integer
            .investigation_budgets
            .logical_answering_requests,
        finite_limit(i64::MAX as u64)
    );

    let trusted =
        ConfigFile::from_toml("[ai]\ninvestigation_max_logical_answering_requests = 5\n").unwrap();
    let project =
        ConfigFile::from_toml("[ai]\ninvestigation_max_logical_answering_requests = 'unlimited'\n")
            .unwrap();
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(trusted)
            .with_project(project),
    )
    .unwrap();
    assert_eq!(
        resolved.investigation_budgets.logical_answering_requests,
        finite_limit(5)
    );
    assert!(
        resolved
            .ignored_project_overrides
            .contains(&"ai.investigation_max_logical_answering_requests".to_owned())
    );

    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::default())
            .with_project(
                ConfigFile::from_toml(
                    "[ai]\ninvestigation_max_logical_answering_requests = 'unlimited'\n",
                )
                .unwrap(),
            )
            .with_cli(CliOverrides {
                trust_project_config: true,
                ..Default::default()
            }),
    )
    .unwrap();
    assert_eq!(
        resolved.investigation_budgets.logical_answering_requests,
        BudgetLimit::Unlimited
    );
}
