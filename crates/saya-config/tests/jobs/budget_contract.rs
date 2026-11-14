use super::*;

/// The resolved run ceilings convert into the run contract's `Budgets` shape —
/// the type the engine layers RunSpec and step budgets over — and the result
/// passes the contract's own validation.
#[test]
fn resolved_run_limits_convert_into_contract_budgets_that_validate() {
    let resolved = resolve_with_user(
        "[limits.run]\nturns = 40\ntool_calls = 25\nwall_clock_seconds = 1800\n\
         [limits.run.tokens_per_endpoint]\n\"local-ollama\" = 200_000\n",
    );
    let budgets = resolved.limits.run.budgets();
    budgets
        .validate()
        .expect("resolved run limits satisfy the run contract");
    assert_eq!(budgets.turns, Some(40));
    assert_eq!(budgets.tool_calls, Some(25));
    assert_eq!(budgets.wall_clock, Some(Duration::from_secs(1800)));
    assert_eq!(
        budgets.tokens_per_endpoint.get("local-ollama"),
        Some(&200_000)
    );

    // With nothing declared, the conversion carries no ceiling at all.
    let default_budgets = resolve(ResolutionInput::new(ConnectionsFile::default()))
        .expect("resolution succeeds")
        .limits
        .run
        .budgets();
    default_budgets
        .validate()
        .expect("the default run limits satisfy the run contract");
    assert_eq!(default_budgets.turns, None);
    assert_eq!(default_budgets.tool_calls, None);
    assert_eq!(default_budgets.wall_clock, None);
    assert!(default_budgets.tokens_per_endpoint.is_empty());
}

/// Zero is meaningless on every run ceiling: zero turns or zero tool calls
/// pause a run before it does anything, and a zero-second wall clock is the
/// same instant pause. Each is rejected at resolve time with a typed error
/// naming the floor, matching the `context_byte_budget` discipline.
#[test]
fn run_limits_below_one_are_rejected_naming_the_key() {
    for (toml, key) in [
        ("[limits.run]\nturns = 0\n", "limits.run.turns"),
        ("[limits.run]\ntool_calls = 0\n", "limits.run.tool_calls"),
        (
            "[limits.run]\nwall_clock_seconds = 0\n",
            "limits.run.wall_clock_seconds",
        ),
    ] {
        let error = resolve(
            ResolutionInput::new(ConnectionsFile::default())
                .with_user(ConfigFile::from_toml(toml).expect("fixture must parse")),
        )
        .unwrap_err();
        assert!(
            matches!(&error, ConfigError::InvalidLimit { key: field } if *field == key),
            "expected InvalidLimit for {toml:?}, got {error:?}"
        );
    }
}

/// `[limits.run] tokens_per_endpoint` values are token ceilings, and zero has
/// the same degenerate meaning: every run pauses the moment the endpoint
/// spends a token. Rejected like its sibling dimensions.
#[test]
fn a_zero_token_ceiling_is_rejected() {
    let error = resolve(
        ResolutionInput::new(ConnectionsFile::default()).with_user(
            ConfigFile::from_toml("[limits.run.tokens_per_endpoint]\nollama = 0\n")
                .expect("fixture must parse"),
        ),
    )
    .unwrap_err();
    assert!(
        matches!(
            error,
            ConfigError::SettingBelowMinimum {
                value: 0,
                min: 1,
                ..
            }
        ),
        "expected SettingBelowMinimum for a zero token ceiling, got {error:?}"
    );
}

/// The token map is bounded exactly like the run contract's budget shape
/// (`MAX_BUDGET_ENDPOINTS`), so a config the contract would reject cannot
/// resolve.
#[test]
fn more_than_the_contract_endpoint_count_is_rejected() {
    let entries = (0..9)
        .map(|i| format!("endpoint-{i} = 1000"))
        .collect::<Vec<_>>()
        .join("\n");
    let toml = format!("[limits.run.tokens_per_endpoint]\n{entries}\n");
    let error = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml(&toml).expect("fixture must parse")),
    )
    .unwrap_err();
    assert!(
        matches!(
            error,
            ConfigError::SettingAboveMaximum {
                value: 9,
                max: 8,
                ..
            }
        ),
        "expected SettingAboveMaximum for nine endpoints, got {error:?}"
    );
}

/// An endpoint key must have the run-scoped name shape the contracts demand;
/// a key the contract rejects at plan-validation time must be caught at
/// resolve time instead, far from the config mistake.
#[test]
fn an_endpoint_key_outside_the_name_shape_is_rejected() {
    let error = resolve(
        ResolutionInput::new(ConnectionsFile::default()).with_user(
            ConfigFile::from_toml("[limits.run.tokens_per_endpoint]\n\"two words\" = 1000\n")
                .expect("fixture must parse"),
        ),
    )
    .unwrap_err();
    assert!(
        matches!(&error, ConfigError::InvalidEndpointName { key, .. } if key == "two words"),
        "expected InvalidEndpointName for a whitespace key, got {error:?}"
    );
}
