use super::*;

/// A run with nothing declared has no turn ceiling: `turns` is opt-in like
/// every other budget dimension, and a ceiling left unset is unlimited at
/// the contract level. `[jobs] turns` still resolves when declared.
#[test]
fn jobs_turns_unset_means_unlimited() {
    let resolved =
        resolve(ResolutionInput::new(ConnectionsFile::default())).expect("resolution succeeds");
    assert_eq!(
        resolved.jobs.turns, None,
        "an undeclared turn ceiling is unlimited, not the max_iterations default"
    );
    assert_eq!(
        resolved.jobs.budgets().turns,
        None,
        "the resolved budget carries the unlimited turns through"
    );

    let declared = resolve_with_user("[jobs]\nturns = 5\n");
    assert_eq!(declared.jobs.turns, Some(5));
    assert_eq!(declared.jobs.budgets().turns, Some(5));

    let other_ceiling = resolve_with_user("[run]\nmax_iterations = 30\n");
    assert_eq!(
        other_ceiling.jobs.turns, None,
        "max_iterations no longer becomes a run's turn ceiling"
    );
}

/// The other budget dimensions are unchanged: wall-clock, tool calls, and
/// token ceilings resolve exactly as before.
#[test]
fn jobs_other_budget_dimensions_are_unchanged() {
    let resolved =
        resolve(ResolutionInput::new(ConnectionsFile::default())).expect("resolution succeeds");
    let jobs = &resolved.jobs;
    assert_eq!(
        jobs.wall_clock_seconds, None,
        "no wall-clock ceiling by default"
    );
    assert_eq!(jobs.tool_calls, None, "no tool-call ceiling by default");
    assert!(
        jobs.tokens_per_endpoint.is_empty(),
        "no token ceilings by default: {:?}",
        jobs.tokens_per_endpoint
    );

    let declared = resolve_with_user(
        "[jobs]\nturns = 40\ntool_calls = 25\nwall_clock_seconds = 1800\n\
         [jobs.tokens_per_endpoint]\n\"local-ollama\" = 200_000\n",
    );
    assert_eq!(declared.jobs.turns, Some(40));
    assert_eq!(declared.jobs.tool_calls, Some(25));
    assert_eq!(declared.jobs.wall_clock_seconds, Some(1800));
    assert_eq!(
        declared.jobs.tokens_per_endpoint,
        BTreeMap::from([("local-ollama".into(), 200_000)])
    );
}

/// A declared `[jobs] turns` resolves verbatim, independent of
/// `[run] max_iterations`: the per-dimension layering rule every budget
/// follows.
#[test]
fn jobs_turn_ceiling_is_independent_of_run_max_iterations() {
    let resolved = resolve_with_user("[run]\nmax_iterations = 30\n[jobs]\nturns = 7\n");
    assert_eq!(resolved.jobs.turns, Some(7));
}

/// Zero is meaningless on every budget default: zero turns or zero tool calls
/// pause a run before it does anything, and a zero-second wall clock is the
/// same instant pause. Each is rejected at resolve time with a typed error
/// naming the floor, matching the `context_byte_budget` discipline.
#[test]
fn jobs_below_one_is_rejected_naming_the_field() {
    for (toml, field) in [
        ("[jobs]\nturns = 0\n", "turns"),
        ("[jobs]\ntool_calls = 0\n", "tool_calls"),
        ("[jobs]\nwall_clock_seconds = 0\n", "wall_clock_seconds"),
    ] {
        let error = resolve(
            ResolutionInput::new(ConnectionsFile::default())
                .with_user(ConfigFile::from_toml(toml).expect("fixture must parse")),
        )
        .unwrap_err();
        assert!(
            matches!(error, ConfigError::SettingBelowMinimum { min: 1, .. }),
            "expected SettingBelowMinimum for {toml:?}, got {error:?}"
        );
        let display = format!("{error}");
        assert!(
            display.contains(field),
            "error must name the field {field:?}: {display}"
        );
    }
}

/// `[jobs] tokens_per_endpoint` values are token ceilings, and zero has the
/// same degenerate-default meaning: every run pauses the moment the endpoint
/// spends a token. Rejected like its sibling budgets.
#[test]
fn jobs_zero_token_ceiling_is_rejected() {
    let error = resolve(
        ResolutionInput::new(ConnectionsFile::default()).with_user(
            ConfigFile::from_toml("[jobs.tokens_per_endpoint]\nollama = 0\n")
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

/// `[run] max_iterations` no longer feeds any run behaviour, but the stored
/// setting stays range-checked: zero would read as a ceiling that pauses
/// everything, a typo, not an intent.
#[test]
fn max_iterations_below_one_is_rejected_even_without_a_reader() {
    let error = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml("[run]\nmax_iterations = 0\n").unwrap()),
    )
    .unwrap_err();
    assert!(
        matches!(
            error,
            ConfigError::SettingBelowMinimum {
                field: "max_iterations",
                value: 0,
                min: 1
            }
        ),
        "expected SettingBelowMinimum for max_iterations = 0, got {error:?}"
    );
}

/// The token map is bounded exactly like the run contract's budget shape
/// (`MAX_BUDGET_ENDPOINTS`), so a config the contract would reject cannot
/// resolve.
#[test]
fn jobs_more_than_the_contract_endpoint_count_is_rejected() {
    let entries = (0..9)
        .map(|i| format!("endpoint-{i} = 1000"))
        .collect::<Vec<_>>()
        .join("\n");
    let toml = format!("[jobs.tokens_per_endpoint]\n{entries}\n");
    let error = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml(&toml).expect("fixture must parse")),
    )
    .unwrap_err();
    assert!(
        matches!(
            error,
            ConfigError::SettingAboveMaximum {
                field: "tokens_per_endpoint",
                value: 9,
                max: 8
            }
        ),
        "expected SettingAboveMaximum for nine endpoints, got {error:?}"
    );
}

/// An endpoint key must have the run-scoped name shape the contracts demand;
/// a key the contract rejects at plan-validation time must be caught at
/// resolve time instead, far from the config mistake.
#[test]
fn jobs_endpoint_key_outside_the_name_shape_is_rejected() {
    let error = resolve(
        ResolutionInput::new(ConnectionsFile::default()).with_user(
            ConfigFile::from_toml("[jobs.tokens_per_endpoint]\n\"two words\" = 1000\n")
                .expect("fixture must parse"),
        ),
    )
    .unwrap_err();
    assert!(
        matches!(&error, ConfigError::InvalidEndpointName { key, .. } if key == "two words"),
        "expected InvalidEndpointName for a whitespace key, got {error:?}"
    );
}
